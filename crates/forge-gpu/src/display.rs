//! What the OS says of a display (issue #94): whether it shows HDR, the white it gives SDR
//! content, and the panel's luminances. Vulkan has no query for these.
//!
//! On Windows: the display configuration API (`QueryDisplayConfig`,
//! `DisplayConfigGetDeviceInfo`) for the HDR switch ("Use HDR") and the SDR white level (the
//! "SDR content brightness" slider), and DXGI's `IDXGIOutput6::GetDesc1` for the luminances,
//! called through its interface table by hand (`windows-sys` has no COM interfaces). Elsewhere
//! nothing is known.

/// What the OS reports of one display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayCaps {
    /// Whether the OS shows this display in HDR (Windows' "Use HDR").
    pub hdr_on: bool,
    /// Whether the display can show HDR at all.
    pub hdr_supported: bool,
    /// The nits the OS gives SDR white on this display in HDR (Windows' "SDR content
    /// brightness"; 80 nits times its level).
    pub sdr_white: f32,
    /// The panel's peak luminance in nits, from its EDID through DXGI, when known.
    pub peak: Option<f32>,
    /// The panel's peak over the whole screen in nits, when known.
    pub full_frame_peak: Option<f32>,
    /// The panel's black in nits, when known.
    pub black: Option<f32>,
}

impl DisplayCaps {
    /// What the OS says of the display named `gdi_name` (Windows' `\\.\DISPLAYn`, as winit's
    /// `MonitorHandle::native_id` gives it). `None` when the OS says nothing (other systems,
    /// or no display of that name).
    pub fn query(gdi_name: &str) -> Option<Self> {
        #[cfg(windows)]
        {
            windows::query(gdi_name)
        }
        #[cfg(not(windows))]
        {
            let _ = gdi_name;
            None
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    use windows_sys::Win32::Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
        DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
        QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
    };
    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LUID};

    use super::DisplayCaps;

    pub(super) fn query(gdi_name: &str) -> Option<DisplayCaps> {
        let path = active_paths()
            .into_iter()
            .find(|path| source_name(path).is_some_and(|name| name == gdi_name))?;
        let (hdr_supported, hdr_on) = advanced_color(&path).unwrap_or((false, false));
        let sdr_white = sdr_white(&path).unwrap_or(80.0);
        let luminance = dxgi::luminance(gdi_name);
        Some(DisplayCaps {
            hdr_on,
            hdr_supported,
            sdr_white,
            peak: luminance.map(|l| l.peak),
            full_frame_peak: luminance.map(|l| l.full_frame_peak),
            black: luminance.map(|l| l.black),
        })
    }

    fn active_paths() -> Vec<DISPLAYCONFIG_PATH_INFO> {
        let (mut path_count, mut mode_count) = (0_u32, 0_u32);
        // SAFETY: plain out-parameters.
        if unsafe {
            GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
        } != ERROR_SUCCESS
        {
            return Vec::new();
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        // SAFETY: the arrays hold the counts passed; QDC_ONLY_ACTIVE_PATHS takes no topology.
        let result = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        };
        if result != ERROR_SUCCESS {
            return Vec::new();
        }
        paths.truncate(path_count as usize);
        paths
    }

    fn header<T>(kind: i32, adapter: LUID, id: u32) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
        DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: kind,
            size: std::mem::size_of::<T>() as u32,
            adapterId: adapter,
            id,
        }
    }

    fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
        let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: header::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(
                DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                path.sourceInfo.adapterId,
                path.sourceInfo.id,
            ),
            ..Default::default()
        };
        // SAFETY: the header starts a request of the size it states.
        if unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } != 0 {
            return None;
        }
        Some(wide_to_string(&request.viewGdiDeviceName))
    }

    fn advanced_color(path: &DISPLAYCONFIG_PATH_INFO) -> Option<(bool, bool)> {
        let mut request = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
            header: header::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>(
                DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                path.targetInfo.adapterId,
                path.targetInfo.id,
            ),
            ..Default::default()
        };
        // SAFETY: as above.
        if unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } != 0 {
            return None;
        }
        // SAFETY: the union's `value` covers its bitfield; every bit pattern is a valid u32.
        let bits = unsafe { request.Anonymous.value };
        // Bit 0: advancedColorSupported, bit 1: advancedColorEnabled.
        Some((bits & 1 != 0, bits & 2 != 0))
    }

    fn sdr_white(path: &DISPLAYCONFIG_PATH_INFO) -> Option<f32> {
        let mut request = DISPLAYCONFIG_SDR_WHITE_LEVEL {
            header: header::<DISPLAYCONFIG_SDR_WHITE_LEVEL>(
                DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                path.targetInfo.adapterId,
                path.targetInfo.id,
            ),
            SDRWhiteLevel: 0,
        };
        // SAFETY: as above.
        if unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } != 0 {
            return None;
        }
        // In thousandths of 80 nits.
        (request.SDRWhiteLevel > 0).then(|| request.SDRWhiteLevel as f32 / 1000.0 * 80.0)
    }

    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    /// DXGI through its interface tables, for `IDXGIOutput6::GetDesc1` alone.
    mod dxgi {
        use super::*;

        #[derive(Clone, Copy)]
        pub(super) struct Luminance {
            pub peak: f32,
            pub full_frame_peak: f32,
            pub black: f32,
        }

        #[repr(C)]
        struct Guid(u32, u16, u16, [u8; 8]);

        const IID_IDXGI_FACTORY1: Guid = Guid(
            0x770a_ae78,
            0xf26f,
            0x4dba,
            [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
        );
        const IID_IDXGI_OUTPUT6: Guid = Guid(
            0x0683_46e8,
            0xaaec,
            0x4b84,
            [0xad, 0xd7, 0x13, 0x7f, 0x51, 0x3f, 0x77, 0xa1],
        );

        /// `DXGI_OUTPUT_DESC1`.
        #[repr(C)]
        struct OutputDesc1 {
            device_name: [u16; 32],
            desktop: [i32; 4],
            attached: i32,
            rotation: u32,
            monitor: *mut c_void,
            bits_per_color: u32,
            color_space: u32,
            primaries: [f32; 8],
            min_luminance: f32,
            max_luminance: f32,
            max_full_frame_luminance: f32,
        }

        #[link(name = "dxgi")]
        unsafe extern "system" {
            fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
        }

        // Slots in the interface tables (dxgi.h, dxgi1_6.h): IUnknown's three, IDXGIObject's
        // four, then each interface's own in order.
        const QUERY_INTERFACE: usize = 0;
        const RELEASE: usize = 2;
        const FACTORY1_ENUM_ADAPTERS1: usize = 12;
        const ADAPTER_ENUM_OUTPUTS: usize = 7;
        const OUTPUT6_GET_DESC1: usize = 27;

        /// A COM pointer released on drop.
        struct Com(*mut c_void);

        impl Com {
            /// The function in slot `slot` of the object's table.
            ///
            /// # Safety
            /// The object is live and its table has a function at `slot`.
            unsafe fn slot(&self, slot: usize) -> *const c_void {
                // SAFETY: a COM object starts with a pointer to its table of functions.
                unsafe { *(*(self.0 as *const *const *const c_void)).add(slot) }
            }
        }

        impl Drop for Com {
            fn drop(&mut self) {
                // SAFETY: IUnknown::Release on a live object this wrapper holds a reference to.
                unsafe {
                    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                        std::mem::transmute(self.slot(RELEASE));
                    release(self.0);
                }
            }
        }

        /// The luminances of the output named `gdi_name`.
        pub(super) fn luminance(gdi_name: &str) -> Option<Luminance> {
            let mut factory = std::ptr::null_mut();
            // SAFETY: a valid IID and out-pointer.
            if unsafe { CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut factory) } < 0 {
                return None;
            }
            let factory = Com(factory);
            for a in 0.. {
                let mut adapter = std::ptr::null_mut();
                // SAFETY: IDXGIFactory1::EnumAdapters1(UINT, IDXGIAdapter1**) on a live factory.
                let result = unsafe {
                    let enum_adapters: unsafe extern "system" fn(
                        *mut c_void,
                        u32,
                        *mut *mut c_void,
                    ) -> i32 = std::mem::transmute(factory.slot(FACTORY1_ENUM_ADAPTERS1));
                    enum_adapters(factory.0, a, &mut adapter)
                };
                if result < 0 {
                    return None;
                }
                let adapter = Com(adapter);
                for o in 0.. {
                    let mut output = std::ptr::null_mut();
                    // SAFETY: IDXGIAdapter::EnumOutputs(UINT, IDXGIOutput**) on a live adapter.
                    let result = unsafe {
                        let enum_outputs: unsafe extern "system" fn(
                            *mut c_void,
                            u32,
                            *mut *mut c_void,
                        )
                            -> i32 = std::mem::transmute(adapter.slot(ADAPTER_ENUM_OUTPUTS));
                        enum_outputs(adapter.0, o, &mut output)
                    };
                    if result < 0 {
                        break;
                    }
                    let output = Com(output);
                    let mut output6 = std::ptr::null_mut();
                    // SAFETY: IUnknown::QueryInterface on a live output.
                    let result = unsafe {
                        let query: unsafe extern "system" fn(
                            *mut c_void,
                            *const Guid,
                            *mut *mut c_void,
                        ) -> i32 = std::mem::transmute(output.slot(QUERY_INTERFACE));
                        query(output.0, &IID_IDXGI_OUTPUT6, &mut output6)
                    };
                    if result < 0 {
                        continue;
                    }
                    let output6 = Com(output6);
                    // SAFETY: zeroes are a valid `DXGI_OUTPUT_DESC1` (a null monitor handle).
                    let mut desc: OutputDesc1 = unsafe { std::mem::zeroed() };
                    // SAFETY: IDXGIOutput6::GetDesc1(DXGI_OUTPUT_DESC1*) on a live output.
                    let result = unsafe {
                        let get_desc1: unsafe extern "system" fn(
                            *mut c_void,
                            *mut OutputDesc1,
                        ) -> i32 = std::mem::transmute(output6.slot(OUTPUT6_GET_DESC1));
                        get_desc1(output6.0, &mut desc)
                    };
                    if result >= 0 && wide_to_string(&desc.device_name) == gdi_name {
                        return Some(Luminance {
                            peak: desc.max_luminance,
                            full_frame_peak: desc.max_full_frame_luminance,
                            black: desc.min_luminance,
                        });
                    }
                }
            }
            None
        }
    }
}
