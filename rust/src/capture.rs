use anyhow::{Context, Result};
use image::RgbaImage;
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
        Imaging::SoftwareBitmap,
        SizeInt32,
    },
    Storage::Streams::{Buffer, DataReader},
    Win32::{
        Foundation::{HMODULE, LPARAM, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device,
            },
            Dwm::DwmFlush,
            Dxgi::IDXGIDevice,
            Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
                CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors,
                GetDC, GetDIBits, HDC, HGDIOBJ, HMONITOR, ReleaseDC, SRCCOPY, SelectObject,
            },
        },
        System::WinRT::{
            Direct3D11::CreateDirect3D11DeviceFromDXGIDevice,
            Graphics::Capture::IGraphicsCaptureItemInterop, RO_INIT_MULTITHREADED, RoInitialize,
            RoUninitialize,
        },
    },
    core::{BOOL, Interface},
};
use windows_future::AsyncStatus;

pub struct Capture {
    pub image: RgbaImage,
    pub origin: [i32; 2],
    pub monitors: Vec<[i32; 4]>,
}

struct CaptureRequest {
    flush_desktop: bool,
    result: Sender<Result<Capture>>,
    wake: Box<dyn FnOnce() + Send>,
}

pub struct CaptureWorker(Sender<CaptureRequest>);

impl CaptureWorker {
    pub fn new() -> Result<Self> {
        let (requests, queue) = mpsc::channel::<CaptureRequest>();
        thread::Builder::new()
            .name("ThodShot capture".into())
            .spawn(move || {
                let apartment = Apartment::initialize();
                let device = apartment.0.then(device).and_then(Result::ok);
                while let Ok(request) = queue.recv() {
                    if request.flush_desktop {
                        let _ = unsafe { DwmFlush() };
                    }
                    let _ = request.result.send(capture(device.as_ref()));
                    (request.wake)();
                }
            })?;
        Ok(Self(requests))
    }

    pub fn request(
        &self,
        flush_desktop: bool,
        wake: impl FnOnce() + Send + 'static,
    ) -> Result<Receiver<Result<Capture>>> {
        let (result, receiver) = mpsc::channel();
        self.0
            .send(CaptureRequest {
                flush_desktop,
                result,
                wake: Box::new(wake),
            })
            .map_err(|_| anyhow::anyhow!("Поток захвата остановился"))?;
        Ok(receiver)
    }
}

struct Apartment(bool);

impl Apartment {
    fn initialize() -> Self {
        Self(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                RoUninitialize();
            }
        }
    }
}

const FRAME_BUDGET: Duration = Duration::from_millis(90);

fn capture(device: Option<&IDirect3DDevice>) -> Result<Capture> {
    let screens = monitors()?;
    anyhow::ensure!(!screens.is_empty(), "Нет доступных мониторов");
    let left = screens.iter().map(|(_, r)| r.left).min().unwrap();
    let top = screens.iter().map(|(_, r)| r.top).min().unwrap();
    let right = screens.iter().map(|(_, r)| r.right).max().unwrap();
    let bottom = screens.iter().map(|(_, r)| r.bottom).max().unwrap();
    let mut image =
        (screens.len() > 1).then(|| RgbaImage::new((right - left) as u32, (bottom - top) as u32));
    // Start the frame pools together so a slow display cannot delay every other monitor.
    let sessions: Vec<_> = screens
        .iter()
        .map(|&(monitor, bounds)| {
            device.and_then(|device| WgcSession::start(device, monitor, bounds).ok())
        })
        .collect();
    let deadline = Instant::now() + FRAME_BUDGET;
    let mut monitors = Vec::new();
    for ((_, bounds), session) in screens.into_iter().zip(sessions) {
        let width = (bounds.right - bounds.left) as u32;
        let height = (bounds.bottom - bounds.top) as u32;
        let shot = session
            .and_then(|session| session.frame(deadline).ok())
            .filter(|shot| {
                if shot.dimensions() != (width, height) {
                    return false;
                }
                let mut dark = 0;
                let mut total = 0;
                for y in (0..shot.height()).step_by((shot.height() / 40).max(1) as usize) {
                    for x in (0..shot.width()).step_by((shot.width() / 40).max(1) as usize) {
                        let p = shot.get_pixel(x, y);
                        total += 1;
                        if p[0] <= 2 && p[1] <= 2 && p[2] <= 2 {
                            dark += 1;
                        }
                    }
                }
                dark * 100 <= total * 98
            });
        let shot = match shot {
            Some(shot) => shot,
            None => gdi(bounds)?,
        };
        let (x, y) = (bounds.left - left, bounds.top - top);
        if let Some(canvas) = &mut image {
            image::imageops::replace(canvas, &shot, i64::from(x), i64::from(y));
        } else {
            image = Some(shot);
        }
        monitors.push([x, y, width as i32, height as i32]);
    }
    Ok(Capture {
        image: image.context("Нет кадра захвата")?,
        origin: [left, top],
        monitors,
    })
}

fn monitors() -> Result<Vec<(HMONITOR, RECT)>> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let screens = unsafe { &mut *(data.0 as *mut Vec<(HMONITOR, RECT)>) };
        screens.push((monitor, unsafe { *rect }));
        true.into()
    }
    let mut screens = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut screens as *mut _ as isize),
        )
        .ok()?;
    }
    Ok(screens)
}

fn gdi(bounds: RECT) -> Result<RgbaImage> {
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let mut bytes = vec![0; width as usize * height as usize * 4];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let screen = GetDC(None);
        anyhow::ensure!(!screen.is_invalid(), "GetDC");
        let target = CreateCompatibleDC(Some(screen));
        let result = (|| -> Result<()> {
            anyhow::ensure!(!target.is_invalid(), "CreateCompatibleDC");
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            anyhow::ensure!(!bitmap.is_invalid(), "CreateCompatibleBitmap");
            let previous = SelectObject(target, HGDIOBJ(bitmap.0));
            let result = if previous.is_invalid() {
                Err(anyhow::anyhow!("SelectObject"))
            } else {
                let copied = BitBlt(
                    target,
                    0,
                    0,
                    width,
                    height,
                    Some(screen),
                    bounds.left,
                    bounds.top,
                    SRCCOPY | CAPTUREBLT,
                );
                SelectObject(target, previous);
                copied.map_err(anyhow::Error::from).and_then(|()| {
                    let rows = GetDIBits(
                        target,
                        bitmap,
                        0,
                        height as u32,
                        Some(bytes.as_mut_ptr().cast()),
                        &mut info,
                        DIB_RGB_COLORS,
                    );
                    anyhow::ensure!(rows == height, "GetDIBits");
                    Ok(())
                })
            };
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            result
        })();
        if !target.is_invalid() {
            let _ = DeleteDC(target);
        }
        ReleaseDC(None, screen);
        result?;
    }
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    RgbaImage::from_raw(width as u32, height as u32, bytes).context("Неверный размер кадра GDI")
}

fn device() -> Result<IDirect3DDevice> {
    let mut device: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    let dxgi: IDXGIDevice = device.context("D3D11 device")?.cast()?;
    Ok(unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?)
}

struct WgcSession {
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl WgcSession {
    fn start(device: &IDirect3DDevice, monitor: HMONITOR, bounds: RECT) -> Result<Self> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor)? };
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            SizeInt32 {
                Width: bounds.right - bounds.left,
                Height: bounds.bottom - bounds.top,
            },
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let capture = Self { pool, session };
        let _ = capture.session.SetIsCursorCaptureEnabled(false);
        capture.session.StartCapture()?;
        Ok(capture)
    }

    fn frame(&self, deadline: Instant) -> Result<RgbaImage> {
        let frame = loop {
            if let Ok(frame) = self.pool.TryGetNextFrame() {
                break frame;
            }
            wait_for_frame(deadline)?;
        };
        let readback = SoftwareBitmap::CreateCopyFromSurfaceAsync(&frame.Surface()?)?;
        while readback.Status()? == AsyncStatus::Started {
            if let Err(error) = wait_for_frame(deadline) {
                let _ = readback.Cancel();
                return Err(error);
            }
        }
        let bitmap = readback.GetResults()?;
        let width = bitmap.PixelWidth()? as u32;
        let height = bitmap.PixelHeight()? as u32;
        let buffer = Buffer::Create(width * height * 4)?;
        bitmap.CopyToBuffer(&buffer)?;
        let mut bytes = vec![0; (width * height * 4) as usize];
        DataReader::FromBuffer(&buffer)?.ReadBytes(&mut bytes)?;
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        bitmap.Close()?;
        frame.Close()?;
        RgbaImage::from_raw(width, height, bytes).context("WGC frame size")
    }
}

impl Drop for WgcSession {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn wait_for_frame(deadline: Instant) -> Result<()> {
    let left = deadline.saturating_duration_since(Instant::now());
    anyhow::ensure!(!left.is_zero(), "WGC timeout");
    thread::sleep(left.min(Duration::from_millis(2)));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn desktop_capture() {
        let worker = super::CaptureWorker::new().unwrap();
        for attempt in 1..=4 {
            let start = std::time::Instant::now();
            let capture = worker
                .request(false, || {})
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap()
                .unwrap();
            eprintln!("capture {attempt}: {:?}", start.elapsed());
            assert!(capture.image.width() > 0 && capture.image.height() > 0);
            assert!(!capture.monitors.is_empty());
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn gdi_capture_matches_physical_monitor_bounds() {
        for (_, bounds) in super::monitors().unwrap() {
            let image = super::gdi(bounds).unwrap();
            assert_eq!(
                image.dimensions(),
                (
                    (bounds.right - bounds.left) as u32,
                    (bounds.bottom - bounds.top) as u32
                )
            );
            assert!(image.pixels().all(|pixel| pixel[3] == 255));
        }
    }
}
