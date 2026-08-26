//! Hardware video decode capability.
//!
//! A driver listing a decoder profile is not proof it can be used: the
//! profile is only usable if the adapter also accepts an output format we can
//! sample from. NV12 is the format the zero-copy path needs, so every profile
//! is confirmed with CheckVideoDecoderFormat before being reported.

use windows::core::GUID;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11VideoDevice, D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0,
    D3D11_DECODER_PROFILE_H264_VLD_NOFGT, D3D11_DECODER_PROFILE_HEVC_VLD_MAIN,
    D3D11_DECODER_PROFILE_HEVC_VLD_MAIN10, D3D11_DECODER_PROFILE_VP9_VLD_PROFILE0,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_FORMAT_P010};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeCaps {
    pub h264: bool,
    pub hevc_main: bool,
    pub hevc_main10: bool,
    pub vp9: bool,
    pub av1: bool,
}

impl DecodeCaps {
    /// Nothing plays without at least one hardware decoder — CPU decode is
    /// not an option (see CLAUDE.md).
    pub fn any(&self) -> bool {
        self.h264 || self.hevc_main || self.hevc_main10 || self.vp9 || self.av1
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.h264 {
            parts.push("h264");
        }
        if self.hevc_main {
            parts.push("hevc");
        }
        if self.hevc_main10 {
            parts.push("hevc10");
        }
        if self.vp9 {
            parts.push("vp9");
        }
        if self.av1 {
            parts.push("av1");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("+")
        }
    }
}

pub fn probe(video: &ID3D11VideoDevice) -> DecodeCaps {
    unsafe {
        let supported: Vec<GUID> = (0..video.GetVideoDecoderProfileCount())
            .filter_map(|i| video.GetVideoDecoderProfile(i).ok())
            .collect();

        // 8-bit profiles decode to NV12; 10-bit ones to P010. Checking a
        // 10-bit profile against NV12 reports "unsupported" on hardware that
        // handles it perfectly well.
        let has = |profile: GUID, format: DXGI_FORMAT| -> bool {
            supported.contains(&profile)
                && video
                    .CheckVideoDecoderFormat(&profile, format)
                    .map(|ok| ok.as_bool())
                    .unwrap_or(false)
        };

        DecodeCaps {
            h264: has(D3D11_DECODER_PROFILE_H264_VLD_NOFGT, DXGI_FORMAT_NV12),
            hevc_main: has(D3D11_DECODER_PROFILE_HEVC_VLD_MAIN, DXGI_FORMAT_NV12),
            hevc_main10: has(D3D11_DECODER_PROFILE_HEVC_VLD_MAIN10, DXGI_FORMAT_P010),
            vp9: has(D3D11_DECODER_PROFILE_VP9_VLD_PROFILE0, DXGI_FORMAT_NV12),
            av1: has(D3D11_DECODER_PROFILE_AV1_VLD_PROFILE0, DXGI_FORMAT_NV12),
        }
    }
}
