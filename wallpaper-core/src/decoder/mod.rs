//! Turning a file on disk into something the shader can sample.
//!
//! Two kinds, and the difference matters for what this project promises:
//!
//! - **Video** (`video.rs`) decodes on the GPU through Media Foundation and
//!   never touches system memory. That is the hard rule; see CLAUDE.md.
//! - **Images and GIFs** (`still.rs`) decode on the CPU through WIC, because
//!   no GPU decodes a PNG and there is nothing to decode per frame anyway. A
//!   still image is uploaded once and then costs nothing at all — it is the
//!   cheapest wallpaper Muivly can show, not an exception to the rule.

mod still;
mod video;

use std::path::Path;
use std::time::Duration;

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView,
};

pub use still::is_still;

/// A frame ready to sample, in whichever form its decoder produces.
///
/// NV12 comes straight from the video decoder and is converted in the shader.
/// BGRA is what WIC hands back. Keeping them apart rather than converting one
/// into the other is the whole point: a conversion would be a per-frame pass
/// over every pixel for no visible gain.
pub enum Frame {
    Nv12 {
        luma: ID3D11ShaderResourceView,
        chroma: ID3D11ShaderResourceView,
        width: u32,
        height: u32,
    },
    Bgra {
        view: ID3D11ShaderResourceView,
        width: u32,
        height: u32,
    },
}

impl Frame {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Frame::Nv12 { width, height, .. } | Frame::Bgra { width, height, .. } => {
                (*width, *height)
            }
        }
    }
}

/// One open wallpaper, whatever kind it turned out to be.
pub enum Wallpaper {
    Video(video::VideoDecoder),
    Still(still::StillDecoder),
}

impl Wallpaper {
    /// Open a file, picking the decoder from its extension.
    ///
    /// `max_scale` caps how large a frame the decoder is asked to produce. A
    /// 4K clip on a 1440p screen is otherwise decoded at 4K and thrown away
    /// on the way to the shader — three times the memory and the bandwidth
    /// for pixels no one can see.
    pub fn open(
        device: &ID3D11Device,
        path: &Path,
        max_scale: (u32, u32),
    ) -> windows::core::Result<Self> {
        let wallpaper = if still::is_still(path) {
            Wallpaper::Still(still::StillDecoder::open(device, path, max_scale)?)
        } else {
            Wallpaper::Video(video::VideoDecoder::open(device, path, max_scale)?)
        };

        // Worth a line in the log: it is the one place the scale cap can be
        // seen working, and "why is a 4K clip using so much memory" is the
        // question it answers in a bug report.
        let (width, height) = wallpaper.frame().size();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!("decode: {name} at {width}x{height}");

        Ok(wallpaper)
    }

    pub fn frame(&self) -> Frame {
        match self {
            Wallpaper::Video(d) => d.frame(),
            Wallpaper::Still(d) => d.frame(),
        }
    }

    /// How many times the wallpaper has played through. A playlist advances
    /// on this. A still image never ends, so it counts elapsed time against
    /// a nominal length instead — see `still.rs`.
    pub fn loops(&self) -> u32 {
        match self {
            Wallpaper::Video(d) => d.loops(),
            Wallpaper::Still(d) => d.loops(),
        }
    }

    /// Play faster or slower than the file was authored at.
    pub fn set_speed(&mut self, speed: f32) {
        match self {
            Wallpaper::Video(d) => d.set_speed(speed),
            Wallpaper::Still(d) => d.set_speed(speed),
        }
    }

    /// How long until this wallpaper has a new frame to show.
    pub fn time_to_next(&self) -> Duration {
        match self {
            Wallpaper::Video(d) => d.time_to_next(),
            Wallpaper::Still(d) => d.time_to_next(),
        }
    }

    /// Advance to `elapsed`. True when a new frame was made current.
    pub fn update(
        &mut self,
        context: &ID3D11DeviceContext,
        elapsed: Duration,
    ) -> windows::core::Result<bool> {
        match self {
            Wallpaper::Video(d) => d.update(context, elapsed),
            Wallpaper::Still(d) => d.update(context, elapsed),
        }
    }
}

/// Fit `size` inside `max` without changing its shape. Returns `size`
/// unchanged when it already fits.
///
/// Rounded to even numbers because NV12 stores chroma at half resolution in
/// both axes, and an odd dimension has no half.
pub fn clamp_size(size: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    if size.0 == 0 || size.1 == 0 || (size.0 <= max.0 && size.1 <= max.1) {
        return size;
    }

    let by_width = max.0 as f64 / size.0 as f64;
    let by_height = max.1 as f64 / size.1 as f64;
    let scale = by_width.min(by_height);

    let even = |v: f64| ((v.round() as u32).max(2)) & !1;
    (even(size.0 as f64 * scale), even(size.1 as f64 * scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_that_fits_is_left_alone() {
        assert_eq!(clamp_size((1920, 1080), (2560, 1440)), (1920, 1080));
    }

    #[test]
    fn four_k_on_a_1440p_budget_keeps_its_shape() {
        assert_eq!(clamp_size((3840, 2160), (2560, 1440)), (2560, 1440));
    }

    #[test]
    fn an_ultrawide_is_capped_by_its_long_axis() {
        // 32:9 inside a 16:9 box: the width is what runs out first, and the
        // height must come down with it rather than being stretched.
        let (w, h) = clamp_size((5120, 1440), (2560, 1440));
        assert_eq!(w, 2560);
        assert_eq!(h, 720);
    }

    #[test]
    fn a_portrait_clip_is_capped_by_its_height() {
        let (w, h) = clamp_size((2160, 3840), (2560, 1440));
        assert_eq!(h, 1440);
        assert_eq!(w, 810);
    }

    #[test]
    fn dimensions_stay_even_for_nv12() {
        let (w, h) = clamp_size((1999, 1001), (1000, 1000));
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }

    #[test]
    fn a_zero_sized_frame_does_not_divide_by_zero() {
        assert_eq!(clamp_size((0, 0), (1920, 1080)), (0, 0));
    }
}
