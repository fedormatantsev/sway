//! `FrameSequence` — a folder of images published as one array texture.
//!
//! Loading is not the material's job (design D7): this node owns the folder,
//! the ordering and the assembly, and `SpriteMaterial` receives the finished
//! texture over a wire. One node type serves both the colour run and the depth
//! run, which is why the colour space is a field here rather than something
//! inferred from where the sequence is wired (design D8).

use bevy::asset::{AssetEvent, AssetId, LoadedFolder, RenderAssetUsages};
use bevy::image::TextureFormatPixelInfo;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::WgpuLimits;
use sway_graph::EditorPos;

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

/// A run of images loaded from one folder and published as a single layered
/// texture, one layer per image.
///
/// **Filenames must be zero-padded** — `000.png`, `001.png`, … `010.png`.
/// Order is ascending by filename and is deliberately lexicographic, so an
/// unpadded run sorts `10.png` *before* `9.png` and the animation plays in the
/// wrong order with nothing to report. The alternative (parsing digits out of
/// filenames) invents an authoring convention the folder does not state;
/// padding is the convention every sprite-sheet exporter already writes.
///
/// The folder is the authoring unit (design D7): a sequence has one path
/// field, and the layer count is derived from what actually loaded rather than
/// authored, so a partly-loaded sequence can never be sampled out of range.
#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default, PartialEq)]
#[require(FrameSequenceOut, FrameSequenceState, EditorPos)]
pub struct FrameSequence {
    pub folder: String,
    pub color_space: ColorSpace,
}

/// The outlet, in the sense of architecture §2: an entity is a frame-sequence
/// producer because it has one of these. Not authorable — a handle has no
/// business round-tripping through a document.
///
/// `layers` is published beside the texture because the read side's clamp
/// (design D4) is bounded by the sequence's *actual* layer count, and a
/// consumer cannot ask a `Handle<Image>` how many layers it has without
/// resolving it.
#[derive(Component, Reflect, Default, Debug, Clone, PartialEq)]
#[reflect(Component, Default, PartialEq)]
pub struct FrameSequenceOut {
    pub texture: Handle<Image>,
    pub layers: u32,
}

/// Per-node loading state.
///
/// **DEVIATION from the task brief**, which specified
/// `#[require(FrameSequenceOut, EditorPos)]`: the strong `Handle<LoadedFolder>`
/// has to live somewhere, because dropping it unloads the folder *and* every
/// frame in it, and it cannot live in `FrameSequenceOut` — that is the wire's
/// payload and holds exactly what a consumer needs. Requiring it keeps the
/// system free of `Commands` and of the one-tick gap a deferred insert would
/// open. It is deliberately not `Reflect`, so it is invisible to the document
/// and to the inspector; the authorable surface is still `FrameSequence` alone.
#[derive(Component, Default, Debug)]
pub struct FrameSequenceState {
    /// The folder path this state was built for. Compared against the node's
    /// own field so that editing the *colour space* re-assembles without
    /// restarting the folder load.
    folder_path: String,
    folder: Handle<LoadedFolder>,
    /// Set when something that could change the outcome happened; cleared when
    /// an assembly is attempted. This is the whole anti-spam mechanism — see
    /// [`sync_frame_sequences`].
    dirty: bool,
    /// The last diagnostic reported for this node, so a permanent error is
    /// logged once rather than once per attempt.
    reported: Option<String>,
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
/// The obvious reading of D8 — "loads its frames via `load_with_settings`,
/// setting `is_srgb` from it" — does not compose with `load_folder` in Bevy
/// 0.19, and the reason is not a missing overload but a silent one:
/// `load_folder` starts a load for every file it finds, so by the time the
/// paths are known, each frame already has an `AssetInfo`. A later
/// `load_with_settings` on the same path reaches
/// `AssetInfos::get_or_create_path_handle_internal`, hits the *occupied* arm,
/// and returns the existing handle — the `MetaTransform` carrying `is_srgb` is
/// dropped on the floor with no warning (bevy_asset 0.19,
/// `server/info.rs`). The settings would appear to be applied and would not be.
/// It is also the wrong unit of control: the asset is keyed by path, so the
/// same folder wired as a colour run on one node and a data run on another
/// would fight over one shared interpretation.
///
/// So the folder is enumerated with `load_folder`, and the colour space is
/// applied where it is unambiguous — on the assembled array texture, by
/// choosing its `TextureFormat` (see [`assemble_layers`]). This is
/// byte-identical to the `load_with_settings` route for the 8-bit content D8
/// specifies, and it lets one folder serve both runs.
///
/// **Not-ready state.** Nothing is published until every frame has resolved in
/// `Assets<Image>`; a partly-arrived folder writes no texture and reports
/// nothing, because a frame still in flight is not an error.
///
/// **Idempotence under `file_watcher`** (which `sway-app` enables — see
/// `headless.rs`). The buffer is rebuilt from the folder's frames every time,
/// so a reload replaces layers instead of appending them. Handle discipline
/// follows `sync_pbr_materials`: allocate only when the outlet still holds
/// `Handle::default()` — never "when the asset is missing", because
/// `ImagePlugin` seeds a real fallback `Image` at the default handle and
/// `get_mut` would happily overwrite the engine's shared default — and
/// otherwise mutate the published asset in place, so every material already
/// holding this handle picks the reload up without the handle changing.
///
/// **Reporting once, not 120 times a second.** `GraphDiagnostics` is not the
/// mechanism here: it is rebuilt at authoring time from the wire graph and has
/// no vocabulary for an asset that failed to load. So a failure is an
/// `error!`, kept from repeating by two independent brakes: an assembly is
/// only attempted when something changed (the node, its folder asset, or one
/// of its frames — all message-driven), and the message itself is compared
/// against the last one reported for that node, so even a retry storm logs a
/// permanent failure once.
pub fn sync_frame_sequences(
    asset_server: Res<AssetServer>,
    folders: Res<Assets<LoadedFolder>>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
    mut folder_messages: MessageReader<AssetEvent<LoadedFolder>>,
    mut image_messages: MessageReader<AssetEvent<Image>>,
    mut nodes: Query<(
        Ref<FrameSequence>,
        &mut FrameSequenceOut,
        &mut FrameSequenceState,
    )>,
) {
    let touched_folders: Vec<AssetId<LoadedFolder>> =
        folder_messages.read().filter_map(changed_id).collect();
    // The published texture is never one of these: it is not in any folder, so
    // this crate's own writes cannot re-trigger an assembly and spin.
    let touched_images: Vec<AssetId<Image>> =
        image_messages.read().filter_map(changed_id).collect();

    // Design D7's ceiling. `RenderDevice` is reachable from the main world in
    // Bevy 0.19 — `RenderResources::unpack_into` does
    // `main_world.insert_resource(device.clone())` before handing the same
    // device to the render world — so this is the real adapter limit in the
    // app (`sway-gpu` requests `adapter.limits()` wholesale, so typically 2048
    // on desktop). It is `Option` only because a device-free test app has no
    // such resource; there the honest bound is wgpu's own default of 256,
    // which is the floor any device is guaranteed to meet.
    let max_layers = device
        .map(|device| device.limits().max_texture_array_layers)
        .unwrap_or_else(|| WgpuLimits::default().max_texture_array_layers);

    for (node, mut out, mut state) in &mut nodes {
        if node.folder.is_empty() {
            // What a palette click produces before anyone types a path. Asking
            // the asset server to enumerate "" logs an error of its own.
            continue;
        }

        if state.folder_path != node.folder {
            state.folder_path = node.folder.clone();
            state.folder = asset_server.load_folder(&node.folder);
            state.dirty = true;
            state.reported = None;
        } else if node.is_changed() {
            // The colour space changed without the folder changing: the frames
            // are still the right ones, only their interpretation moved.
            state.dirty = true;
        }

        let folder = folders.get(&state.folder);
        if touched_folders.contains(&state.folder.id()) {
            state.dirty = true;
        }
        if let Some(folder) = folder
            && !touched_images.is_empty()
            && folder
                .handles
                .iter()
                .any(|handle| touched_images.iter().any(|id| *id == handle.id().typed()))
        {
            state.dirty = true;
        }

        if !state.dirty {
            continue;
        }
        // Cleared before the attempt, not after: an attempt that finds frames
        // still in flight must not retry every frame — the message that
        // announces the missing frame's arrival is what wakes it up again.
        state.dirty = false;

        let Some(folder) = folder else {
            continue;
        };

        let mut frames: Vec<(String, Handle<Image>)> = Vec::with_capacity(folder.handles.len());
        for handle in &folder.handles {
            let name = handle
                .path()
                .map(|path| path.path().to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Ok(image) = handle.clone().try_typed::<Image>() {
                frames.push((name, image));
            }
        }
        sort_frames_by_name(&mut frames);

        // Borrowing `images` immutably ends with this block, which is what lets
        // the assembled image be written back into the same collection below.
        let assembled = {
            let mut resolved: Vec<(&str, &Image)> = Vec::with_capacity(frames.len());
            let mut ready = true;
            for (name, handle) in &frames {
                match images.get(handle) {
                    Some(image) => resolved.push((name.as_str(), image)),
                    // Spec: a sequence is unavailable until every frame has
                    // loaded. Not an error, and nothing is published.
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            assemble_layers(&resolved, node.color_space, max_layers)
        };

        let assembled = match assembled {
            Ok(image) => image,
            Err(error) => {
                let message = format!("frame sequence '{}' {error}", node.folder);
                if state.reported.as_deref() != Some(message.as_str()) {
                    error!("{message}");
                    state.reported = Some(message);
                }
                // Publish nothing rather than something wrong: a material whose
                // run is unavailable renders nothing, which is the visible
                // failure the spec asks for.
                out.set_if_neq(FrameSequenceOut::default());
                continue;
            }
        };
        state.reported = None;

        let layers = assembled.texture_descriptor.size.depth_or_array_layers;
        let texture = out.texture.clone();
        if texture != Handle::default()
            && let Some(mut published) = images.get_mut(&texture)
        {
            *published = assembled;
            out.set_if_neq(FrameSequenceOut { texture, layers });
        } else {
            out.set_if_neq(FrameSequenceOut {
                texture: images.add(assembled),
                layers,
            });
        }
    }
}

/// Which asset messages can change an assembly's outcome. `Removed`/`Unused`
/// cannot: they only ever arrive for assets nothing holds a handle to, and this
/// node holds strong handles to every frame it uses.
///
/// `pub(crate)` because `sprite_material.rs`'s bounds system asks the same
/// question of `AssetEvent<Mesh>` — "did this asset's contents change" — and a
/// second copy of the match would be a second place to forget a variant.
pub(crate) fn changed_id<A: Asset>(message: &AssetEvent<A>) -> Option<AssetId<A>> {
    match message {
        AssetEvent::Added { id }
        | AssetEvent::Modified { id }
        | AssetEvent::LoadedWithDependencies { id } => Some(*id),
        AssetEvent::Removed { .. } | AssetEvent::Unused { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

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

    /// `AssetPlugin` plus the one asset type, as `mesh_asset.rs` does — no
    /// device, no renderer. `ImagePlugin` seeds a real fallback `Image` at
    /// `Handle::default()` in every real app, so seed it here too: without it
    /// this app cannot catch a sync system that only calls `images.get_mut` and
    /// never allocates.
    fn frame_sequence_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), AssetPlugin::default()));
        app.init_asset::<Image>();
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&Handle::default(), Image::default())
            .expect("seeding the default handle succeeds");
        app.add_systems(Update, sync_frame_sequences);
        app
    }

    /// Spawns a sequence whose folder is already enumerated.
    ///
    /// The state component is seeded rather than letting the system call
    /// `load_folder`, so the test needs no assets directory and no async
    /// arrival: `folder_path` already matches, so the system takes the folder
    /// handed to it here. Everything after enumeration — readiness, ordering,
    /// assembly, handle discipline — is the real code path.
    fn spawn_sequence(app: &mut App, frames: Vec<Handle<Image>>) -> Entity {
        let folder = app
            .world_mut()
            .resource_mut::<Assets<LoadedFolder>>()
            .add(LoadedFolder {
                handles: frames.into_iter().map(Handle::untyped).collect(),
            });
        app.world_mut()
            .spawn((
                FrameSequence {
                    folder: "frames".into(),
                    color_space: ColorSpace::Display,
                },
                FrameSequenceState {
                    folder_path: "frames".into(),
                    folder,
                    dirty: true,
                    reported: None,
                },
            ))
            .id()
    }

    #[test]
    fn require_supplies_the_outlet_a_wire_writes_from() {
        let mut app = frame_sequence_app();
        let entity = app.world_mut().spawn(FrameSequence::default()).id();

        assert!(app.world().get::<FrameSequenceOut>(entity).is_some());
        assert!(app.world().get::<EditorPos>(entity).is_some());
    }

    #[test]
    fn an_empty_folder_path_publishes_nothing() {
        // What a palette click produces before anyone types a path. It must not
        // ask the asset server to enumerate "", which logs an error every tick.
        let mut app = frame_sequence_app();
        let entity = app.world_mut().spawn(FrameSequence::default()).id();

        app.update();

        assert_eq!(
            app.world().get::<FrameSequenceOut>(entity),
            Some(&FrameSequenceOut::default())
        );
    }

    #[test]
    fn a_sequence_publishes_nothing_until_every_frame_has_loaded() {
        // Spec scenario "A sequence is unavailable until every frame has
        // loaded". The failure this catches is publishing an array assembled
        // from whichever frames happen to have arrived — a short sequence whose
        // layer count is a race.
        let mut app = frame_sequence_app();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let arrived = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 1));
        let in_flight = images.reserve_handle();
        let entity = spawn_sequence(&mut app, vec![arrived, in_flight.clone()]);

        app.update();

        assert_eq!(
            app.world().get::<FrameSequenceOut>(entity),
            Some(&FrameSequenceOut::default()),
            "one frame is still in flight"
        );

        // The remaining frame arrives, which is itself the message that wakes
        // the assembly up — nothing polls.
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&in_flight, frame(2, 2, TextureFormat::Rgba8UnormSrgb, 2))
            .expect("insert succeeds");
        app.update();

        let out = app
            .world()
            .get::<FrameSequenceOut>(entity)
            .expect("required");
        assert_eq!(out.layers, 2);
        assert_ne!(out.texture, Handle::default());
    }

    #[test]
    fn a_reload_mutates_the_published_texture_in_place_rather_than_thrashing_the_handle() {
        // `sway-app` runs with `file_watcher` on, so a saved frame re-enters
        // this system as a `Modified` message. Two failures are covered: a
        // fresh handle per reload (every consuming material would have to be
        // rewired, and the old texture leaks until the handle drops), and an
        // assembly that appends to the previous buffer instead of rebuilding it
        // (the layer count would climb on every save).
        let mut app = frame_sequence_app();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let first = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 1));
        let second = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 2));
        let entity = spawn_sequence(&mut app, vec![first.clone(), second]);

        app.update();
        let before = app
            .world()
            .get::<FrameSequenceOut>(entity)
            .expect("required")
            .clone();

        // A frame is edited on disk and reloaded in place, as `file_watcher`
        // does.
        *app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&first)
            .expect("present") = frame(2, 2, TextureFormat::Rgba8UnormSrgb, 9);
        app.update();

        let after = app
            .world()
            .get::<FrameSequenceOut>(entity)
            .expect("required");
        assert_eq!(after.texture, before.texture, "the handle must not change");
        assert_eq!(after.layers, 2, "a reload replaces layers, never appends");
        let published = app
            .world()
            .resource::<Assets<Image>>()
            .get(&after.texture)
            .expect("the handle resolves");
        assert_eq!(
            published.data.as_ref().map(|bytes| bytes[0]),
            Some(9),
            "the edit reached the published texture"
        );
    }

    #[test]
    fn a_settled_sequence_stops_reassembling() {
        // The anti-spam property, stated as a test rather than as a comment: a
        // system that reassembles every tick would also `error!` every tick on
        // a permanently broken folder. A settled node leaves its own outlet
        // unchanged, which is what stops the wire downstream from firing too.
        let mut app = frame_sequence_app();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let one = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 1));
        let two = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 2));
        let entity = spawn_sequence(&mut app, vec![one, two]);

        app.update();
        app.update();
        // A third tick with nothing happening: the outlet must be untouched.
        app.update();

        assert!(
            !app.world()
                .entity(entity)
                .get_ref::<FrameSequenceOut>()
                .expect("required")
                .is_changed(),
            "an idle sequence must not rewrite its outlet"
        );
        assert!(
            !app.world()
                .get::<FrameSequenceState>(entity)
                .expect("required")
                .dirty
        );
    }

    #[test]
    fn a_folder_of_mismatched_frames_publishes_no_texture() {
        // Spec scenario "Frames of differing dimensions are rejected": no
        // texture, rather than one assembled out of whatever agreed.
        let mut app = frame_sequence_app();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let good = images.add(frame(4, 4, TextureFormat::Rgba8UnormSrgb, 1));
        let odd = images.add(frame(2, 2, TextureFormat::Rgba8UnormSrgb, 2));
        let entity = spawn_sequence(&mut app, vec![good, odd]);

        app.update();

        assert_eq!(
            app.world().get::<FrameSequenceOut>(entity),
            Some(&FrameSequenceOut::default()),
            "nothing is published for a folder that cannot be assembled"
        );
        assert!(
            app.world()
                .get::<FrameSequenceState>(entity)
                .expect("required")
                .reported
                .is_some(),
            "the diagnostic is remembered, so a permanent failure reports once"
        );
    }
}
