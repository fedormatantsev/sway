//! `FrameSequence`: a folder of images published as one array texture — the
//! node kind, and the ordering and assembly it does.
//!
//! Loading is not the material's job (design D7): this node owns the folder,
//! the ordering and the assembly, and `SpriteMaterial` receives the finished
//! texture over a connection. One node type serves both the colour run and the
//! depth run, which is why the colour space is a field here rather than
//! something inferred from where the sequence is wired (design D8).
//!
//! The node owns its texture. Nothing hands it along a connection: an edge
//! from `outlets.sequence` carries a ZST and exists only to say the
//! connection is there and to order the two projectors (design D6).

use bevy::asset::{AssetEvent, AssetId, RenderAssetUsages};
use bevy::image::TextureFormatPixelInfo;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

/// How the stored bytes are meant to be read (design D8). One node type serves
/// both runs, so this cannot be inferred from the node — a depth run read
/// through a display transfer curve would warp the depth mapping, and a colour
/// run read as data would render dark.
#[derive(Reflect, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    /// Colour: the stored value carries a display (sRGB) transfer curve, and
    /// the sampler undoes it. Equivalent to `ImageLoaderSettings::is_srgb =
    /// true`.
    #[default]
    Display,
    /// Data: the stored value *is* the value. What a depth run needs, and
    /// equivalent to `ImageLoaderSettings::is_srgb = false`.
    Data,
}

/// Why a folder could not be published as one array texture. Every variant
/// names the offending frame, because "some frame is the wrong size" is not an
/// actionable diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceError {
    /// The folder held no images at all.
    NoFrames,
    /// More frames than the device can hold in one array texture.
    TooManyLayers { layers: u32, limit: u32 },
    /// A frame's dimensions disagree with the first frame's.
    MismatchedSize {
        frame: String,
        expected: (u32, u32),
        found: (u32, u32),
    },
    /// A frame's texture format disagrees with the first frame's.
    MismatchedFormat {
        frame: String,
        expected: TextureFormat,
        found: TextureFormat,
    },
    /// A frame is itself layered, mipped, or not 2D — nothing sensible can be
    /// concatenated out of it.
    UnsupportedFrame { frame: String },
    /// A compressed (block) format, which cannot be concatenated row-wise.
    UnsupportedFormat {
        frame: String,
        format: TextureFormat,
    },
    /// A frame carries no CPU-side pixels, or not as many as its own
    /// descriptor claims.
    UnreadableFrame { frame: String },
    /// Bevy's own validation of the stacked-to-array reinterpretation refused.
    Reinterpret(String),
}

impl core::fmt::Display for SequenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoFrames => write!(f, "holds no images"),
            Self::TooManyLayers { layers, limit } => write!(
                f,
                "holds {layers} frames, but the device's maximum texture array layers is {limit}"
            ),
            Self::MismatchedSize {
                frame,
                expected,
                found,
            } => write!(
                f,
                "frame '{frame}' is {}x{}, but the first frame is {}x{}",
                found.0, found.1, expected.0, expected.1
            ),
            Self::MismatchedFormat {
                frame,
                expected,
                found,
            } => write!(
                f,
                "frame '{frame}' is {found:?}, but the first frame is {expected:?}"
            ),
            Self::UnsupportedFrame { frame } => write!(
                f,
                "frame '{frame}' is not a single-layer, single-mip 2D image"
            ),
            Self::UnsupportedFormat { frame, format } => write!(
                f,
                "frame '{frame}' is {format:?}, a block-compressed format that cannot be stacked"
            ),
            Self::UnreadableFrame { frame } => write!(
                f,
                "frame '{frame}' has no readable pixel data on the CPU side"
            ),
            Self::Reinterpret(message) => {
                write!(f, "could not be read as an array texture: {message}")
            }
        }
    }
}

/// The filename part of an asset path. Ordering is by filename rather than by
/// full path so that the folder's own name — which every frame shares — cannot
/// influence it.
fn frame_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Sorts frames ascending by filename.
///
/// Design D7: filesystem enumeration order is arbitrary, so this sort is what
/// makes a sequence deterministic, and it is the one part of this that must be
/// tested rather than trusted. Pure — no GPU, no ECS (architecture §9).
///
/// The full path breaks ties: `AssetServer::load_folder` recurses into
/// subdirectories, so two files can share a filename, and a comparison that
/// ignored the rest of the path would leave their relative order down to
/// enumeration again.
pub fn sort_frames_by_name<T>(frames: &mut [(String, T)]) {
    frames.sort_by(|a, b| {
        frame_name(&a.0)
            .cmp(frame_name(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
}

/// Concatenates same-sized frames into one `D2Array` texture, one layer per
/// frame, and applies the sequence's colour space. Pure — no GPU, no ECS.
///
/// Design D7: the frames are stacked vertically into one buffer and the
/// descriptor work is handed to `Image::reinterpret_stacked_2d_as_array`, so
/// Bevy's own validation is what decides the result is coherent rather than
/// arithmetic written here.
///
/// `max_layers` is the caller's bound (the device's
/// `max_texture_array_layers`); exceeding it is an error rather than a
/// truncation, because a silently short sequence looks like an authoring
/// mistake in the animation instead of a limit that was hit.
pub fn assemble_layers(
    frames: &[(&str, &Image)],
    color_space: ColorSpace,
    max_layers: u32,
) -> Result<Image, SequenceError> {
    let Some((_, first)) = frames.first() else {
        return Err(SequenceError::NoFrames);
    };

    let layers = frames.len() as u32;
    if layers > max_layers {
        return Err(SequenceError::TooManyLayers {
            layers,
            limit: max_layers,
        });
    }

    let (width, height) = (first.width(), first.height());
    let source_format = first.texture_descriptor.format;
    let Ok(pixel_size) = source_format.pixel_size() else {
        return Err(SequenceError::UnsupportedFormat {
            frame: frames[0].0.to_string(),
            format: source_format,
        });
    };
    let frame_bytes = width as usize * height as usize * pixel_size;

    let mut data = Vec::with_capacity(frame_bytes * frames.len());
    for (name, image) in frames {
        let descriptor = &image.texture_descriptor;
        if descriptor.dimension != TextureDimension::D2
            || descriptor.size.depth_or_array_layers != 1
            || descriptor.mip_level_count != 1
        {
            return Err(SequenceError::UnsupportedFrame {
                frame: (*name).to_string(),
            });
        }
        if (image.width(), image.height()) != (width, height) {
            return Err(SequenceError::MismatchedSize {
                frame: (*name).to_string(),
                expected: (width, height),
                found: (image.width(), image.height()),
            });
        }
        if descriptor.format != source_format {
            return Err(SequenceError::MismatchedFormat {
                frame: (*name).to_string(),
                expected: source_format,
                found: descriptor.format,
            });
        }
        match image.data.as_deref() {
            Some(bytes) if bytes.len() == frame_bytes => data.extend_from_slice(bytes),
            _ => {
                return Err(SequenceError::UnreadableFrame {
                    frame: (*name).to_string(),
                });
            }
        }
    }

    // Colour space is applied here, on the assembled texture, rather than per
    // frame — see `sync_frame_sequences` for why the per-frame route is not
    // available. `is_srgb` in Bevy's own loader selects nothing but the
    // `TextureFormat` (`Image::from_dynamic` in bevy_image: every arm picks
    // between `Rgba8UnormSrgb` and `Rgba8Unorm` and hands over the *same*
    // bytes), so choosing the format here is byte-for-byte what
    // `load_with_settings` would have produced.
    let format = match color_space {
        ColorSpace::Display => source_format.add_srgb_suffix(),
        ColorSpace::Data => source_format.remove_srgb_suffix(),
    };

    // MAIN_WORLD is kept alongside RENDER_WORLD deliberately: dropping it
    // would halve the memory a sequence costs, but it would also unload the
    // asset from `Assets<Image>` after extraction, and then a `file_watcher`
    // reload could no longer mutate the published texture in place — every
    // reload would have to allocate a new handle and thrash every consumer.
    let mut image = Image::new(
        Extent3d {
            width,
            height: height * layers,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::default(),
    );

    // Real API note: `reinterpret_stacked_2d_as_array` rejects `layers < 2`
    // (`TextureReinterpretationError::NotEnoughLayers`), but a one-frame folder
    // is legal authoring — a still. For one layer the stacked image is already
    // the array's descriptor, so only the *view* needs saying.
    if layers >= 2 {
        image
            .reinterpret_stacked_2d_as_array(layers)
            .map_err(|error| SequenceError::Reinterpret(format!("{error:?}")))?;
    }

    // wgpu's default view dimension for a `D2` texture with one layer is `D2`,
    // not `D2Array`, so the one-frame case would bind as the wrong type. Naming
    // the view for every layer count keeps a one-frame sequence and a thirty-
    // frame sequence interchangeable at the material's binding.
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });

    Ok(image)
}

/// Loads each `FrameSequence`'s folder and publishes it as one array texture.
///
/// **Colour space, and why `load_folder` is used for enumeration only (D8).**
pub(crate) fn changed_id<A: Asset>(message: &AssetEvent<A>) -> Option<AssetId<A>> {
    match message {
        AssetEvent::Added { id }
        | AssetEvent::Modified { id }
        | AssetEvent::LoadedWithDependencies { id } => Some(*id),
        AssetEvent::Removed { .. } | AssetEvent::Unused { .. } => None,
    }
}

// --- the node kind ---------------------------------------------------

use bevy::asset::LoadedFolder;
use bevy::ecs::world::World;
use sway_graph::graph::{NodeKind, ReflectNodeKind};

use crate::nodes::protocol::{self, ImageSequenceOut, ReflectImageSequenceNode};

/// [`FrameSequence`]'s inlets.
///
/// **Filenames must be zero-padded** — `000.png`, `001.png`, … `010.png`.
/// Order is ascending by filename and deliberately lexicographic.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct FrameSequenceIn {
    pub folder: String,
    /// How the stored bytes are meant to be read. One node kind serves both
    /// the colour run and the depth run, so this cannot be inferred from
    /// where the sequence is connected.
    pub color_space: ColorSpace,
}

/// [`FrameSequence`]'s state. Not authored, not serialized.
#[derive(Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Default, Debug, PartialEq)]
pub struct FrameSequenceState {
    /// The published array texture, or `Handle::default()` while nothing has
    /// been published.
    pub texture: Handle<Image>,
    /// The published texture's actual layer count. Derived from what loaded,
    /// never authored, so a partly-loaded sequence can never be sampled out
    /// of range.
    pub layers: u32,
    /// The folder path `folder` was enumerated for. Compared against the
    /// inlet so that editing the *colour space* re-assembles without
    /// restarting the folder load.
    pub folder_path: String,
    /// The strong folder handle. It has to live somewhere: dropping it
    /// unloads the folder *and* every frame in it.
    pub folder: Handle<LoadedFolder>,
    /// Set when something that could change the outcome happened; cleared
    /// when an assembly is attempted. The anti-spam mechanism — an attempt
    /// that finds frames still in flight must not retry every frame.
    pub pending: bool,
    /// The last diagnostic reported, so a permanent error logs once rather
    /// than once per attempt.
    pub reported: Option<String>,
}

/// A run of images loaded from one folder and published as a single layered
/// texture, one layer per image.
#[derive(Reflect, Default, Debug)]
#[reflect(NodeKind, ImageSequenceNode, Default)]
pub struct FrameSequence {
    pub inlets: FrameSequenceIn,
    pub state: FrameSequenceState,
    pub outlets: ImageSequenceOut,
}

impl NodeKind for FrameSequence {
    /// Nothing: enumerating a folder and assembling an array texture needs
    /// the `AssetServer` and `ResMut<Assets<Image>>`, which `&World` cannot
    /// give. The projector does it.
    fn evaluate(&mut self, _world: &World) {}
}

impl protocol::ImageSequenceNode for FrameSequence {
    fn texture(&self) -> &Handle<Image> {
        &self.state.texture
    }

    fn layers(&self) -> u32 {
        self.state.layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A solid frame, so a layer's contents are identifiable by one byte.
    fn frame(width: u32, height: u32, format: TextureFormat, fill: u8) -> Image {
        let pixel_size = format.pixel_size().expect("an uncompressed test format");
        Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![fill; width as usize * height as usize * pixel_size],
            format,
            RenderAssetUsages::default(),
        )
    }

    fn names<T>(frames: &[(String, T)]) -> Vec<&str> {
        frames.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn frames_sort_ascending_by_filename_whatever_order_they_arrived_in() {
        // The failure this catches: publishing frames in `read_directory`
        // order. That order is the filesystem's, so the animation would play
        // correctly on the machine it was authored on and scramble elsewhere.
        let mut frames = vec![
            ("smoke/003.png".to_string(), 3),
            ("smoke/000.png".to_string(), 0),
            ("smoke/002.png".to_string(), 2),
            ("smoke/010.png".to_string(), 10),
            ("smoke/001.png".to_string(), 1),
        ];
        sort_frames_by_name(&mut frames);
        assert_eq!(
            names(&frames),
            vec![
                "smoke/000.png",
                "smoke/001.png",
                "smoke/002.png",
                "smoke/003.png",
                "smoke/010.png"
            ]
        );
    }

    #[test]
    fn ten_sorts_after_nine_only_when_filenames_are_zero_padded() {
        // The trap `FrameSequence`'s doc comment warns about, nailed down here
        // rather than assumed: this sort is lexicographic, so an author who
        // exports `9.png`/`10.png` gets frame 10 played fourth-from-last. The
        // assertion documents the requirement by making the unpadded ordering
        // explicit — if someone "fixes" this with natural-sort, the padded case
        // below must keep working and this case is the deliberate cost.
        let mut unpadded = vec![("9.png".to_string(), 9), ("10.png".to_string(), 10)];
        sort_frames_by_name(&mut unpadded);
        assert_eq!(
            names(&unpadded),
            vec!["10.png", "9.png"],
            "plain lexicographic ordering puts 10.png before 9.png"
        );

        let mut padded = vec![("009.png".to_string(), 9), ("010.png".to_string(), 10)];
        sort_frames_by_name(&mut padded);
        assert_eq!(
            names(&padded),
            vec!["009.png", "010.png"],
            "zero padding is what makes filename order match frame order"
        );
    }

    #[test]
    fn frames_sharing_a_filename_in_different_subfolders_still_order_deterministically() {
        // `load_folder` recurses, so this is reachable. Without the full-path
        // tiebreak the two would be "equal" and their order would fall back to
        // enumeration — the exact non-determinism the sort exists to remove.
        let mut frames = vec![
            ("run/b/000.png".to_string(), 'b'),
            ("run/a/000.png".to_string(), 'a'),
        ];
        sort_frames_by_name(&mut frames);
        assert_eq!(names(&frames), vec!["run/a/000.png", "run/b/000.png"]);
    }

    #[test]
    fn an_assembled_sequence_is_one_array_texture_with_a_layer_per_frame() {
        // Catches an assembly that stacks the frames but forgets to reinterpret
        // the stack as layers: the texture would be a tall 2D image, and the
        // material would sample a squashed strip instead of one frame.
        let frames = [
            frame(2, 2, TextureFormat::Rgba8UnormSrgb, 10),
            frame(2, 2, TextureFormat::Rgba8UnormSrgb, 20),
            frame(2, 2, TextureFormat::Rgba8UnormSrgb, 30),
        ];
        let borrowed: Vec<(&str, &Image)> = vec![
            ("000.png", &frames[0]),
            ("001.png", &frames[1]),
            ("002.png", &frames[2]),
        ];

        let assembled = assemble_layers(&borrowed, ColorSpace::Display, 256).expect("assembles");

        assert_eq!(assembled.texture_descriptor.size.width, 2);
        assert_eq!(
            assembled.texture_descriptor.size.height, 2,
            "the layer height, not the stacked height"
        );
        assert_eq!(assembled.texture_descriptor.size.depth_or_array_layers, 3);
        let data = assembled.data.as_ref().expect("pixels");
        assert_eq!(data.len(), 3 * 2 * 2 * 4);
        assert_eq!(data[0], 10, "layer 0 holds the first frame");
        assert_eq!(data[2 * 2 * 4], 20, "layer 1 holds the second frame");
        assert_eq!(data[2 * 2 * 2 * 4], 30, "layer 2 holds the third frame");
    }

    #[test]
    fn a_data_run_is_assembled_without_a_display_transfer_curve() {
        // Design D8: a depth run read through sRGB would warp the depth
        // mapping, so the same bytes must land in a non-sRGB format. Both runs
        // are asserted together because the bug is a swap, not an omission.
        let source = frame(1, 1, TextureFormat::Rgba8UnormSrgb, 128);
        let frames: Vec<(&str, &Image)> = vec![("000.png", &source)];

        let colour = assemble_layers(&frames, ColorSpace::Display, 256).expect("assembles");
        assert_eq!(
            colour.texture_descriptor.format,
            TextureFormat::Rgba8UnormSrgb
        );

        let data = assemble_layers(&frames, ColorSpace::Data, 256).expect("assembles");
        assert_eq!(data.texture_descriptor.format, TextureFormat::Rgba8Unorm);
        assert_eq!(
            data.data.as_ref().map(|bytes| bytes[0]),
            Some(128),
            "the stored value is unchanged; only how it is read moved"
        );
    }

    #[test]
    fn a_one_frame_folder_still_publishes_an_array_texture() {
        // `Image::reinterpret_stacked_2d_as_array` refuses fewer than two
        // layers, so the naive implementation returns an error for a legal
        // one-frame sequence — and a `texture_2d_array` binding fed a plain
        // `D2` view is a validation failure at draw time, far from here.
        let source = frame(4, 4, TextureFormat::Rgba8UnormSrgb, 7);
        let frames: Vec<(&str, &Image)> = vec![("000.png", &source)];

        let assembled = assemble_layers(&frames, ColorSpace::Display, 256).expect("assembles");

        assert_eq!(assembled.texture_descriptor.size.depth_or_array_layers, 1);
        assert_eq!(
            assembled
                .texture_view_descriptor
                .as_ref()
                .and_then(|view| view.dimension),
            Some(TextureViewDimension::D2Array)
        );
    }

    #[test]
    fn frames_of_differing_dimensions_are_rejected_and_the_frame_is_named() {
        // Spec scenario "Frames of differing dimensions are rejected". Naming
        // the frame is the point: a 30-frame folder with one stray export is
        // otherwise a hunt.
        let good = frame(4, 4, TextureFormat::Rgba8UnormSrgb, 1);
        let odd = frame(4, 2, TextureFormat::Rgba8UnormSrgb, 1);
        let frames: Vec<(&str, &Image)> = vec![("000.png", &good), ("001.png", &odd)];

        let error = assemble_layers(&frames, ColorSpace::Display, 256).expect_err("rejected");

        assert_eq!(
            error,
            SequenceError::MismatchedSize {
                frame: "001.png".into(),
                expected: (4, 4),
                found: (4, 2),
            }
        );
        assert!(error.to_string().contains("001.png"));
    }

    #[test]
    fn frames_of_differing_formats_are_rejected() {
        // Concatenating an Rgba8 frame with an Rg8 one produces a buffer whose
        // length matches nothing; without this check `Image::new`'s own
        // debug assertion is the first thing to notice, in a debug build only.
        let good = frame(2, 2, TextureFormat::Rgba8UnormSrgb, 1);
        let odd = frame(2, 2, TextureFormat::Rg8Unorm, 1);
        let frames: Vec<(&str, &Image)> = vec![("000.png", &good), ("001.png", &odd)];

        let error = assemble_layers(&frames, ColorSpace::Display, 256).expect_err("rejected");

        assert!(matches!(error, SequenceError::MismatchedFormat { .. }));
        assert!(error.to_string().contains("001.png"));
    }

    #[test]
    fn a_sequence_longer_than_the_device_allows_is_rejected_with_the_limit() {
        // Spec scenario "An oversized sequence is reported": the diagnostic has
        // to carry the limit, or the author cannot tell how much to cut.
        let source = frame(1, 1, TextureFormat::Rgba8UnormSrgb, 1);
        let frames: Vec<(&str, &Image)> = (0..5).map(|_| ("000.png", &source)).collect();

        let error = assemble_layers(&frames, ColorSpace::Display, 4).expect_err("rejected");

        assert_eq!(
            error,
            SequenceError::TooManyLayers {
                layers: 5,
                limit: 4
            }
        );
        assert!(error.to_string().contains('4'));
    }

    #[test]
    fn an_empty_folder_is_reported_rather_than_published_as_a_zero_layer_texture() {
        // A zero-layer texture is not constructible; without this the arithmetic
        // below would produce a zero-height `Extent3d` and wgpu would reject it
        // at upload, far from the folder that caused it.
        let error = assemble_layers(&[], ColorSpace::Display, 256)
            .expect_err("an empty folder is an error");
        assert_eq!(error, SequenceError::NoFrames);
    }
}
