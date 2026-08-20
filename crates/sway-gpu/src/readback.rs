//! Reading a render target back to CPU memory without stalling the frame
//! loop (design D6).
//!
//! Keeping up with the external clock outranks completing a capture, so
//! nothing here waits: a request copies the texture into a buffer taken from a
//! small pool and calls `map_async`; a later frame polls the device
//! *non-blockingly* and collects whatever has finished. There is no
//! `PollType::Wait` on this path, and there must never be one — a blocking
//! poll is the show falling behind the MIDI it is following.
//!
//! Saturation is handled by dropping. When every buffer in the pool is still
//! in flight there is no free one to copy into, so the request is refused and
//! counted; the caller reports the count when the run ends. A recording that
//! loses frames is a worse recording, and a show that misses the clock is a
//! worse show.
//!
//! ## Row padding
//!
//! `copy_texture_to_buffer` requires each row to start on a
//! `COPY_BYTES_PER_ROW_ALIGNMENT` (256) boundary and will not pad for you, so
//! the buffer holds `padded_bytes_per_row` per row while the image wants
//! `width * 4`. [`unpad_rows`] is the one place that difference is undone. A
//! width whose stride is already aligned (1920 x 4 = 7680) hides a mistake
//! here completely, which is why its test uses one that is not.

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, COPY_BYTES_PER_ROW_ALIGNMENT, CommandEncoder,
    CommandEncoderDescriptor, Device, Extent3d, MapMode, Origin3d, PollType, Queue,
    TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureFormat,
};

/// Bytes per pixel of every format this module reads back.
const BYTES_PER_PIXEL: u32 = 4;

/// The padded stride `copy_texture_to_buffer` demands for a row of `width`
/// RGBA8 pixels.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    (width * BYTES_PER_PIXEL).div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Copies `height` rows of `width` RGBA8 pixels out of a padded mapped range,
/// dropping the padding, and swizzles BGRA to RGBA where the source texture
/// was BGRA.
///
/// One pass rather than copy-then-unpad: the bytes have to be copied out of
/// the mapping regardless (the buffer goes straight back into the pool), so
/// unpadding on the way costs nothing extra.
fn unpad_rows(data: &[u8], width: u32, height: u32, bgra: bool) -> Vec<u8> {
    let unpadded = (width * BYTES_PER_PIXEL) as usize;
    let padded = padded_bytes_per_row(width) as usize;
    let mut pixels = Vec::with_capacity(unpadded * height as usize);
    for row in 0..height as usize {
        let start = row * padded;
        pixels.extend_from_slice(&data[start..start + unpadded]);
    }
    if bgra {
        for pixel in pixels.chunks_exact_mut(BYTES_PER_PIXEL as usize) {
            pixel.swap(0, 2);
        }
    }
    pixels
}

/// One completed readback: what was asked for, and the pixels that came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readback {
    /// Whatever the requester passed in — the capture slot's index, for the
    /// capture path, which is also the file's number.
    pub tag: u64,
    pub width: u32,
    pub height: u32,
    /// Unpadded RGBA8, `width * height * 4` bytes, top row first.
    pub pixels: Vec<u8>,
}

/// A readback that could not be issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadbackRefused {
    /// Every buffer in the pool is still in flight. The pool has counted this;
    /// see [`ReadbackPool::dropped`].
    Saturated,
    /// The texture is not one this module can read (not 4 bytes per pixel).
    UnsupportedFormat,
}

/// What a readback needs to remember about the image it is carrying.
#[derive(Clone, Copy)]
struct Shape {
    tag: u64,
    width: u32,
    height: u32,
    bgra: bool,
}

impl Shape {
    fn buffer_size(&self) -> u64 {
        u64::from(padded_bytes_per_row(self.width)) * u64::from(self.height)
    }
}

/// A copy that has been encoded but whose mapping has not been requested yet.
///
/// The two are separate because `map_async` may not be called on a buffer
/// before the copy that fills it has been submitted — wgpu refuses the submit
/// outright with "buffer is still mapped".
struct Encoded {
    buffer: Buffer,
    shape: Shape,
}

/// A request whose mapping has been asked for but has not completed.
struct InFlight {
    buffer: Buffer,
    /// Fires once wgpu has run the `map_async` callback, which only happens
    /// while something polls the device.
    done: Receiver<Result<(), wgpu::BufferAsyncError>>,
    shape: Shape,
}

/// A small fixed set of mappable buffers, recycled across readbacks.
///
/// Fixed rather than growing on demand: an unbounded pool would answer a disk
/// that cannot keep up by consuming memory until the process died, where the
/// specified answer is to drop the slot and say so.
pub struct ReadbackPool {
    device: Device,
    queue: Queue,
    /// Buffers not currently in flight, each remembering the size it was
    /// created at so a request only reallocates when it needs a bigger one.
    free: Vec<Buffer>,
    /// Copies encoded but not yet armed — see [`Encoded`].
    encoded: Vec<Encoded>,
    in_flight: Vec<InFlight>,
    capacity: usize,
    dropped: u64,
}

impl ReadbackPool {
    /// A pool of at most `capacity` concurrent readbacks.
    ///
    /// Buffers are allocated on demand up to that count rather than up front,
    /// because their size is the target's and no target exists yet.
    pub fn new(device: &Device, queue: &Queue, capacity: usize) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            free: Vec::new(),
            encoded: Vec::new(),
            in_flight: Vec::new(),
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Issues a readback of the whole of `texture` on an encoder of its own,
    /// submitted immediately.
    ///
    /// Independent of whether a frame is presentable, which is what a capture
    /// needs: an occluded window must not cost a recording its frames.
    pub fn request(&mut self, texture: &Texture, tag: u64) -> Result<(), ReadbackRefused> {
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("sway readback encoder"),
            });
        let result = self.encode(&mut encoder, texture, tag);
        if result.is_ok() {
            self.queue.submit(Some(encoder.finish()));
            self.arm();
        }
        result
    }

    /// Asks for the mapping of every copy encoded so far.
    ///
    /// Must be called only once the encoder those copies went into has been
    /// submitted: `map_async` before the submit makes wgpu reject the submit
    /// with "buffer is still mapped". [`Self::request`] does this itself, and
    /// [`Self::collect`] does it for copies encoded into a frame — so a
    /// readback encoded through [`crate::Frame::read_back`] must have its
    /// frame presented before the next `collect`.
    pub fn arm(&mut self) {
        for pending in std::mem::take(&mut self.encoded) {
            let (tx, done) = channel();
            pending
                .buffer
                .slice(..pending.shape.buffer_size())
                .map_async(MapMode::Read, move |result| {
                    let _ = tx.send(result);
                });
            self.in_flight.push(InFlight {
                buffer: pending.buffer,
                done,
                shape: pending.shape,
            });
        }
    }

    /// Encodes a readback into an encoder someone else owns and will submit.
    ///
    /// `pub(crate)` so a `wgpu::CommandEncoder` never has to exist outside
    /// this crate; [`crate::Frame::read_back`] is the way in, and it is how
    /// the presented surface texture — which only exists inside a frame — is
    /// captured.
    pub(crate) fn encode(
        &mut self,
        encoder: &mut CommandEncoder,
        texture: &Texture,
        tag: u64,
    ) -> Result<(), ReadbackRefused> {
        let bgra = match texture.format() {
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => false,
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => true,
            _ => return Err(ReadbackRefused::UnsupportedFormat),
        };
        let shape = Shape {
            tag,
            width: texture.width(),
            height: texture.height(),
            bgra,
        };
        let padded = padded_bytes_per_row(shape.width);

        let Some(buffer) = self.take_buffer(shape.buffer_size()) else {
            self.dropped += 1;
            return Err(ReadbackRefused::Saturated);
        };

        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(shape.height),
                },
            },
            Extent3d {
                width: shape.width,
                height: shape.height,
                depth_or_array_layers: 1,
            },
        );

        // The mapping is asked for by `arm`, once this copy has been
        // submitted — never here. See `arm`.
        self.encoded.push(Encoded { buffer, shape });
        Ok(())
    }

    /// A free buffer of at least `size` bytes, or `None` when every buffer the
    /// pool is allowed to own is in flight.
    ///
    /// Never waits for one to come free — that is the whole point.
    fn take_buffer(&mut self, size: u64) -> Option<Buffer> {
        if let Some(index) = self.free.iter().position(|buffer| buffer.size() >= size) {
            return Some(self.free.swap_remove(index));
        }
        if self.free.len() + self.encoded.len() + self.in_flight.len() < self.capacity {
            return Some(self.device.create_buffer(&BufferDescriptor {
                label: Some("sway readback buffer"),
                size,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }
        // Only free buffers left are too small for this target (a resolution
        // grew). Replacing one is allocation, not waiting.
        self.free.pop().map(|_| {
            self.device.create_buffer(&BufferDescriptor {
                label: Some("sway readback buffer"),
                size,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })
    }

    /// Polls the device once, without blocking, and returns every readback
    /// whose mapping has completed since the last call.
    ///
    /// A mapping that failed is dropped and counted rather than reported as an
    /// image: there is nothing to write, and the run's drop count is already
    /// the channel for "this recording is incomplete".
    pub fn collect(&mut self) -> Vec<Readback> {
        // Anything encoded into a caller's own frame has been submitted by
        // now (see `arm`), so this is where those mappings are asked for.
        self.arm();
        // `Poll` — never `Wait`. See the module docs.
        let _ = self.device.poll(PollType::Poll);

        let mut done = Vec::new();
        let mut still_pending = Vec::with_capacity(self.in_flight.len());
        for request in std::mem::take(&mut self.in_flight) {
            match request.done.try_recv() {
                Err(TryRecvError::Empty) => still_pending.push(request),
                Ok(Ok(())) => {
                    let shape = request.shape;
                    let pixels = {
                        let data = request
                            .buffer
                            .slice(..shape.buffer_size())
                            .get_mapped_range();
                        unpad_rows(&data, shape.width, shape.height, shape.bgra)
                    };
                    request.buffer.unmap();
                    done.push(Readback {
                        tag: shape.tag,
                        width: shape.width,
                        height: shape.height,
                        pixels,
                    });
                    self.free.push(request.buffer);
                }
                // A failed mapping, or a sender dropped without sending —
                // neither can yield pixels, and neither may leak the buffer.
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    self.dropped += 1;
                    self.free.push(request.buffer);
                }
            }
        }
        self.in_flight = still_pending;
        done
    }

    /// How many readbacks this pool has refused or lost since it was created.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// How many readbacks are still outstanding — encoded, armed, or both.
    pub fn in_flight(&self) -> usize {
        self.encoded.len() + self.in_flight.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpuContext;
    use wgpu::{TextureDescriptor, TextureDimension, TextureUsages};

    #[test]
    fn a_stride_that_is_already_aligned_is_left_alone() {
        // 1920 x 4 = 7680 = 30 x 256. This is the case that hides a padding
        // bug, which is exactly why it is not the case the unpadding is
        // tested with below.
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn rows_are_unpadded_at_a_width_whose_stride_is_not_aligned() {
        // 1000 px = 4000 bytes, which pads to 4096. A reader that ignored the
        // padding would produce an image skewed by 96 bytes per row — visibly
        // sheared, but never a crash, so only an unaligned width catches it.
        let (width, height) = (1000u32, 3u32);
        assert_eq!(padded_bytes_per_row(width), 4096);

        let padded = padded_bytes_per_row(width) as usize;
        let unpadded = (width * BYTES_PER_PIXEL) as usize;
        let mut data = vec![0u8; padded * height as usize];
        for row in 0..height as usize {
            // Each row is filled with its own row number, and the padding
            // with a value no row uses, so any byte of padding that survives
            // is unmistakable.
            let start = row * padded;
            data[start..start + unpadded].fill(row as u8);
            data[start + unpadded..start + padded].fill(0xEE);
        }

        let pixels = unpad_rows(&data, width, height, false);

        assert_eq!(pixels.len(), unpadded * height as usize);
        assert!(
            !pixels.contains(&0xEE),
            "padding bytes leaked into the image"
        );
        for row in 0..height as usize {
            let start = row * unpadded;
            assert!(
                pixels[start..start + unpadded]
                    .iter()
                    .all(|byte| *byte == row as u8),
                "row {row} is not wholly its own bytes — the rows are skewed"
            );
        }
    }

    #[test]
    fn a_bgra_source_comes_back_as_rgba() {
        // The window surface is `Bgra8Unorm` while every camera target is
        // RGBA. A capture that skipped the swizzle would write a
        // red-and-blue-swapped file that still looks like a plausible image.
        let width = 1u32;
        let mut data = vec![0u8; padded_bytes_per_row(width) as usize];
        data[..4].copy_from_slice(&[10, 20, 30, 40]); // B, G, R, A
        let pixels = unpad_rows(&data, width, 1, true);
        assert_eq!(pixels, vec![30, 20, 10, 40]);
    }

    fn target(device: &Device, width: u32, height: u32) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("readback test target"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    #[test]
    fn a_saturated_pool_refuses_and_counts_instead_of_waiting() {
        // The specified response to a disk (or a GPU) that cannot keep up is
        // to drop the slot and report it. A pool that blocked here would make
        // the show wait on the recording, which is the trade the `runtime`
        // spec settles the other way.
        let gpu = GpuContext::new(None);
        let texture = target(&gpu.device, 8, 8);
        let mut pool = ReadbackPool::new(&gpu.device, &gpu.queue, 2);

        // Two fit; nothing has been polled, so both are still in flight.
        assert_eq!(pool.request(&texture, 0), Ok(()));
        assert_eq!(pool.request(&texture, 1), Ok(()));
        assert_eq!(pool.in_flight(), 2);

        assert_eq!(pool.request(&texture, 2), Err(ReadbackRefused::Saturated));
        assert_eq!(pool.dropped(), 1, "the drop is reported to the caller");
        assert_eq!(pool.in_flight(), 2, "nothing was queued behind the others");
    }

    #[test]
    fn collected_buffers_go_back_into_the_pool() {
        let gpu = GpuContext::new(None);
        let texture = target(&gpu.device, 8, 8);
        let mut pool = ReadbackPool::new(&gpu.device, &gpu.queue, 1);

        pool.request(&texture, 7).expect("the first fits");
        assert_eq!(pool.request(&texture, 8), Err(ReadbackRefused::Saturated));

        // `collect` polls without blocking, so a mapping may need more than
        // one call before it has completed. Bounded so a genuinely stuck
        // mapping fails the test rather than hanging it.
        let mut collected = Vec::new();
        for _ in 0..1000 {
            collected.extend(pool.collect());
            if !collected.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        assert_eq!(collected.len(), 1, "the readback never completed");
        assert_eq!(collected[0].tag, 7);
        assert_eq!(collected[0].pixels.len(), 8 * 8 * 4);
        assert_eq!(pool.in_flight(), 0);
        assert_eq!(
            pool.request(&texture, 9),
            Ok(()),
            "the buffer was returned to the pool"
        );
    }
}
