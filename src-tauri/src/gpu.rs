//! Finds the discrete GPU (most dedicated memory) through DXGI.

use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;

pub struct Adapter {
    /// DXGI enumeration index, which DirectML uses as `device_id`.
    pub index: i32,
    pub name: String,
}

pub fn pick_discrete() -> Option<Adapter> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1().ok()? };
    let mut best: Option<(usize, Adapter)> = None;
    let mut index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        let desc = unsafe { adapter.GetDesc1().ok()? };
        let name_len = desc.Description.iter().position(|&c| c == 0).unwrap_or(0);
        let name = String::from_utf16_lossy(&desc.Description[..name_len]);
        log::info!("DXGI adapter {index}: {name}, {} MiB dedicated", desc.DedicatedVideoMemory >> 20);
        let is_software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE != 0;
        if !is_software && best.as_ref().is_none_or(|(mem, _)| desc.DedicatedVideoMemory > *mem) {
            best = Some((desc.DedicatedVideoMemory, Adapter { index: index as i32, name }));
        }
        index += 1;
    }
    best.map(|(_, adapter)| adapter)
}
