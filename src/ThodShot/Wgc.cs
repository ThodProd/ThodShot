using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
using Windows.Graphics;
using Windows.Graphics.Capture;
using Windows.Graphics.DirectX;
using Windows.Graphics.DirectX.Direct3D11;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace ThodShot
{
    /// <summary>
    /// Захват мониторов через Windows Graphics Capture. В отличие от BitBlt,
    /// он видит содержимое аппаратно ускоренных окон (VMware, браузеры, игры).
    /// </summary>
    internal static class WgcCapture
    {
        [ComImport, Guid("3628E81B-3CAC-4C60-B7F4-23CE0E0C3356"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IGraphicsCaptureItemInterop
        {
            [PreserveSig] int CreateForWindow(IntPtr window, ref Guid iid, out IntPtr item);
            [PreserveSig] int CreateForMonitor(IntPtr monitor, ref Guid iid, out IntPtr item);
        }

        [DllImport("combase.dll")]
        static extern int RoGetActivationFactory(IntPtr activatableClassId, ref Guid iid, out IntPtr factory);
        [DllImport("combase.dll")]
        static extern int WindowsCreateString([MarshalAs(UnmanagedType.LPWStr)] string src, uint len, out IntPtr hstr);
        [DllImport("combase.dll")]
        static extern int WindowsDeleteString(IntPtr hstr);
        [DllImport("d3d11.dll")]
        static extern int D3D11CreateDevice(IntPtr adapter, int driverType, IntPtr software, uint flags,
            IntPtr featureLevels, uint featureLevelsCount, uint sdkVersion, out IntPtr device, out int featureLevel, out IntPtr context);
        [DllImport("d3d11.dll")]
        static extern int CreateDirect3D11DeviceFromDXGIDevice(IntPtr dxgiDevice, out IntPtr device);
        [DllImport("user32.dll")]
        static extern IntPtr MonitorFromPoint(Point pt, uint flags);

        static readonly Lazy<IDirect3DDevice> Device =
            new Lazy<IDirect3DDevice>(CreateDevice, LazyThreadSafetyMode.ExecutionAndPublication);
        static bool unavailable;

        /// <summary>Заранее создаёт D3D-устройство, чтобы первый снимок появлялся быстрее.</summary>
        public static void Warmup()
        {
            try { var d = Device.Value; GC.KeepAlive(d); } catch { }
        }

        /// <summary>Дорисовывает на снимке экрана содержимое мониторов, снятое через WGC.</summary>
        public static void OverlayMonitors(Bitmap canvas, Rectangle virtualBounds)
        {
            if (unavailable || canvas == null) return;
            try
            {
                foreach (var screen in System.Windows.Forms.Screen.AllScreens)
                {
                    Rectangle mb = screen.Bounds;
                    IntPtr mon = MonitorFromPoint(new Point(mb.X + mb.Width / 2, mb.Y + mb.Height / 2), 2);
                    Bitmap shot;
                    if (!TryCapture(mon, mb.Size, out shot)) continue;
                    try
                    {
                        // полностью чёрный кадр обычно означает, что драйвер не отдал содержимое
                        if (IsMostlyBlack(shot)) continue;
                        using (var g = Graphics.FromImage(canvas))
                        {
                            int x = mb.X - virtualBounds.X, y = mb.Y - virtualBounds.Y;
                            g.DrawImage(shot, new Rectangle(x, y, shot.Width, shot.Height),
                                0, 0, shot.Width, shot.Height, GraphicsUnit.Pixel);
                        }
                    }
                    finally { shot.Dispose(); }
                }
            }
            catch (Exception ex) { Log.Write("WGC: " + ex.Message); }
        }

        static bool TryCapture(IntPtr monitor, Size size, out Bitmap bmp)
        {
            bmp = null;
            IDirect3DDevice device;
            try { device = Device.Value; }
            catch (Exception ex)
            {
                unavailable = true;
                Log.Write("WGC: не удалось создать D3D-устройство: " + ex.Message);
                return false;
            }

            GraphicsCaptureItem item = null;
            Direct3D11CaptureFramePool pool = null;
            GraphicsCaptureSession session = null;
            Direct3D11CaptureFrame frame = null;
            try
            {
                item = ItemForMonitor(monitor);
                pool = Direct3D11CaptureFramePool.CreateFreeThreaded(device, DirectXPixelFormat.B8G8R8A8UIntNormalized, 2,
                    new SizeInt32 { Width = size.Width, Height = size.Height });
                session = pool.CreateCaptureSession(item);
                session.IsCursorCaptureEnabled = false;
                session.StartCapture();

                var sw = Stopwatch.StartNew();
                while (frame == null && sw.ElapsedMilliseconds < 1500)
                {
                    frame = pool.TryGetNextFrame();
                    if (frame == null) Thread.Sleep(5);
                }
                if (frame == null) return false;

                using (var sb0 = SoftwareBitmap.CreateCopyFromSurfaceAsync(frame.Surface).AsTask().GetAwaiter().GetResult())
                {
                    SoftwareBitmap sb = sb0;
                    bool converted = false;
                    if (sb.BitmapPixelFormat != BitmapPixelFormat.Bgra8 || sb.BitmapAlphaMode != BitmapAlphaMode.Premultiplied)
                    {
                        sb = SoftwareBitmap.Convert(sb0, BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied);
                        converted = true;
                    }
                    try
                    {
                        int w = sb.PixelWidth, h = sb.PixelHeight;
                        int stride;
                        using (var buffer = sb.LockBuffer(BitmapBufferAccessMode.Read))
                            stride = buffer.GetPlaneDescription(0).Stride;

                        uint total = (uint)(stride * h);
                        var ibuf = new Windows.Storage.Streams.Buffer(total) { Length = total };
                        sb.CopyToBuffer(ibuf);
                        var bytes = new byte[total];
                        using (var reader = DataReader.FromBuffer(ibuf)) reader.ReadBytes(bytes);

                        var result = new Bitmap(w, h, PixelFormat.Format32bppPArgb);
                        var bd = result.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppPArgb);
                        try
                        {
                            int row = Math.Min(stride, bd.Stride);
                            for (int y = 0; y < h; y++)
                                Marshal.Copy(bytes, stride * y, IntPtr.Add(bd.Scan0, bd.Stride * y), row);
                        }
                        finally { result.UnlockBits(bd); }
                        bmp = result;
                    }
                    finally { if (converted) sb.Dispose(); }
                }
                return bmp != null;
            }
            catch (Exception ex)
            {
                Log.Write("WGC: " + ex.Message);
                return false;
            }
            finally
            {
                if (frame != null) frame.Dispose();
                if (session != null) session.Dispose();
                if (pool != null) pool.Dispose();
            }
        }

        static GraphicsCaptureItem ItemForMonitor(IntPtr monitor)
        {
            const string cls = "Windows.Graphics.Capture.GraphicsCaptureItem";
            WindowsCreateString(cls, (uint)cls.Length, out IntPtr hstr);
            Guid iid = new Guid("3628E81B-3CAC-4C60-B7F4-23CE0E0C3356");
            int hr = RoGetActivationFactory(hstr, ref iid, out IntPtr factory);
            WindowsDeleteString(hstr);
            if (hr < 0) throw new Exception("RoGetActivationFactory: 0x" + hr.ToString("X8"));
            try
            {
                var interop = (IGraphicsCaptureItemInterop)Marshal.GetObjectForIUnknown(factory);
                Guid itemIid = new Guid("79C3F95B-31F7-4EC2-A464-632EF5D30760");
                hr = interop.CreateForMonitor(monitor, ref itemIid, out IntPtr ptr);
                if (hr < 0) throw new Exception("CreateForMonitor: 0x" + hr.ToString("X8"));
                try { return WinRT.MarshalInspectable<GraphicsCaptureItem>.FromAbi(ptr); }
                finally { Marshal.Release(ptr); }
            }
            finally { Marshal.Release(factory); }
        }

        static IDirect3DDevice CreateDevice()
        {
            int hr = D3D11CreateDevice(IntPtr.Zero, 1, IntPtr.Zero, 0x20, IntPtr.Zero, 0, 7,
                out IntPtr dev, out int level, out IntPtr ctx);
            if (hr < 0) throw new Exception("D3D11CreateDevice: 0x" + hr.ToString("X8"));
            try
            {
                Guid iid = new Guid("54ec77fa-1377-44e6-8c32-88fd5f44c84c");
                hr = Marshal.QueryInterface(dev, ref iid, out IntPtr dxgi);
                if (hr < 0) throw new Exception("IDXGIDevice: 0x" + hr.ToString("X8"));
                try
                {
                    hr = CreateDirect3D11DeviceFromDXGIDevice(dxgi, out IntPtr inspectable);
                    if (hr < 0) throw new Exception("CreateDirect3D11DeviceFromDXGIDevice: 0x" + hr.ToString("X8"));
                    try { return WinRT.MarshalInterface<IDirect3DDevice>.FromAbi(inspectable); }
                    finally { Marshal.Release(inspectable); }
                }
                finally { Marshal.Release(dxgi); }
            }
            finally
            {
                Marshal.Release(ctx);
                Marshal.Release(dev);
            }
        }

        static bool IsMostlyBlack(Bitmap bmp)
        {
            int stepX = Math.Max(1, bmp.Width / 40), stepY = Math.Max(1, bmp.Height / 40);
            int dark = 0, total = 0;
            for (int y = 0; y < bmp.Height; y += stepY)
                for (int x = 0; x < bmp.Width; x += stepX)
                {
                    Color c = bmp.GetPixel(x, y);
                    total++;
                    if (c.R <= 2 && c.G <= 2 && c.B <= 2) dark++;
                }
            return total > 0 && dark > total * 0.98;
        }
    }
}
