using System;
using System.ComponentModel;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace ThodShot
{
    internal static class ScreenGrab
    {
        public static Bitmap CaptureVirtualScreen(out Rectangle bounds)
        {
            bounds = SystemInformation.VirtualScreen;
            var bmp = new Bitmap(bounds.Width, bounds.Height, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(bmp))
            {
                IntPtr dest = g.GetHdc();
                IntPtr src = Native.GetDC(IntPtr.Zero);
                try
                {
                    // SRCCOPY | CAPTUREBLT: включаем и слоистые окна
                    if (!Native.BitBlt(dest, 0, 0, bounds.Width, bounds.Height, src, bounds.X, bounds.Y, 0x00CC0020u | 0x40000000u))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                }
                finally
                {
                    Native.ReleaseDC(IntPtr.Zero, src);
                    g.ReleaseHdc(dest);
                }
            }
            return bmp;
        }
    }

    /// <summary>Затемнённый оверлей выбора: область / весь экран / отмена.</summary>
    internal sealed class CaptureOverlay : Form
    {
        const int WM_KEYDOWN = 0x0100;
        const int WM_DPICHANGED = 0x02E0;

        readonly Bitmap shot;
        readonly Bitmap dimmed;
        readonly Bitmap vignette;
        readonly Rectangle vb;
        readonly float sc;
        readonly Rectangle[] btn = new Rectangle[3];
        Rectangle bar;
        int hover = -1;
        string mode;
        bool dragging, finished, delivered;
        Point dragStart;
        Rectangle sel;
        long presentedAt;

        public event Action<Bitmap> Completed;
        public event Action Cancelled;

        public CaptureOverlay(Bitmap shot, Rectangle virtualBounds, string defaultMode)
        {
            this.shot = shot;
            vb = virtualBounds;
            mode = defaultMode == "full" ? "full" : "region";

            AutoScaleMode = AutoScaleMode.None;
            FormBorderStyle = FormBorderStyle.None;
            StartPosition = FormStartPosition.Manual;
            ShowInTaskbar = false;
            TopMost = true;
            DoubleBuffered = true;
            BackColor = Color.Black;
            Cursor = Cursors.Cross;
            Bounds = vb;
            sc = Math.Max(1f, DeviceDpi / 96f);

            int w = shot.Width, h = shot.Height;
            dimmed = new Bitmap(w, h, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(dimmed))
            {
                g.DrawImage(shot, new Rectangle(0, 0, w, h), 0, 0, w, h, GraphicsUnit.Pixel);
                using (var br = new SolidBrush(Color.FromArgb(96, 0, 0, 0))) g.FillRectangle(br, 0, 0, w, h);
            }
            vignette = BuildVignette(w, h);
            LayoutToolbar();
            if (mode == "full") sel = MonitorRect(PointToClientSafe(Cursor.Position));
        }

        /// <summary>Тёмная дымка, усиливающаяся к краям экрана.</summary>
        static Bitmap BuildVignette(int w, int h)
        {
            var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb);
            int band = Math.Max(120, Math.Min(w, h) / 4);
            using (var g = Graphics.FromImage(bmp))
            {
                using (var b = new LinearGradientBrush(new Rectangle(0, 0, w, band),
                    Color.FromArgb(112, 0, 0, 0), Color.FromArgb(0, 0, 0, 0), LinearGradientMode.Vertical))
                { b.WrapMode = WrapMode.TileFlipXY; g.FillRectangle(b, 0, 0, w, band); }
                using (var b = new LinearGradientBrush(new Rectangle(0, h - band, w, band),
                    Color.FromArgb(0, 0, 0, 0), Color.FromArgb(135, 0, 0, 0), LinearGradientMode.Vertical))
                { b.WrapMode = WrapMode.TileFlipXY; g.FillRectangle(b, 0, h - band, w, band); }
                using (var b = new LinearGradientBrush(new Rectangle(0, 0, band, h),
                    Color.FromArgb(102, 0, 0, 0), Color.FromArgb(0, 0, 0, 0), LinearGradientMode.Horizontal))
                { b.WrapMode = WrapMode.TileFlipXY; g.FillRectangle(b, 0, 0, band, h); }
                using (var b = new LinearGradientBrush(new Rectangle(w - band, 0, band, h),
                    Color.FromArgb(0, 0, 0, 0), Color.FromArgb(122, 0, 0, 0), LinearGradientMode.Horizontal))
                { b.WrapMode = WrapMode.TileFlipXY; g.FillRectangle(b, w - band, 0, band, h); }
            }
            return bmp;
        }

        protected override CreateParams CreateParams
        {
            get
            {
                var cp = base.CreateParams;
                cp.ExStyle |= 0x80; // WS_EX_TOOLWINDOW: без записи в Alt+Tab
                return cp;
            }
        }

        public void Present()
        {
            Show();
            Native.SetWindowPos(Handle, new IntPtr(-1), vb.X, vb.Y, vb.Width, vb.Height, 0x0040);
            Native.ForceForeground(Handle);
            Activate();
            Focus();
            presentedAt = Environment.TickCount64;
        }

        Point PointToClientSafe(Point screenPt) { return new Point(screenPt.X - vb.X, screenPt.Y - vb.Y); }

        Rectangle MonitorRect(Point formPt)
        {
            Rectangle r = Screen.FromPoint(new Point(formPt.X + vb.X, formPt.Y + vb.Y)).Bounds;
            r.Offset(-vb.X, -vb.Y);
            return r;
        }

        void LayoutToolbar()
        {
            Rectangle mon = MonitorRect(PointToClientSafe(Cursor.Position));
            int bs = (int)(44 * sc), gap = (int)(4 * sc), pad = (int)(6 * sc);
            int w = pad * 2 + bs * 3 + gap * 2, h = bs + pad * 2;
            int x = mon.X + (mon.Width - w) / 2, y = mon.Y + (int)(18 * sc);
            bar = new Rectangle(x, y, w, h);
            for (int i = 0; i < 3; i++) btn[i] = new Rectangle(x + pad + i * (bs + gap), y + pad, bs, bs);
        }

        int HitButton(Point p)
        {
            for (int i = 0; i < 3; i++) if (btn[i].Contains(p)) return i;
            return -1;
        }

        static GraphicsPath Round(Rectangle r, int rad)
        {
            int d = rad * 2;
            var p = new GraphicsPath();
            p.AddArc(r.X, r.Y, d, d, 180, 90);
            p.AddArc(r.Right - d, r.Y, d, d, 270, 90);
            p.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
            p.AddArc(r.X, r.Bottom - d, d, d, 90, 90);
            p.CloseFigure();
            return p;
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            var g = e.Graphics;
            int w = shot.Width, h = shot.Height;
            g.DrawImage(dimmed, new Rectangle(0, 0, w, h), 0, 0, w, h, GraphicsUnit.Pixel);
            g.DrawImage(vignette, new Rectangle(0, 0, w, h), 0, 0, w, h, GraphicsUnit.Pixel);

            if (sel.Width > 0 && sel.Height > 0)
            {
                DrawSelectionShadow(g, sel);
                g.DrawImage(shot, sel, sel.X, sel.Y, sel.Width, sel.Height, GraphicsUnit.Pixel);
                g.SmoothingMode = SmoothingMode.AntiAlias;
                using (var pen = new Pen(Color.FromArgb(54, 118, 232), Math.Max(2f, sc * 1.6f)))
                    g.DrawRectangle(pen, sel.X, sel.Y, sel.Width - 1, sel.Height - 1);
                using (var pen = new Pen(Color.FromArgb(110, 255, 255, 255), 1f))
                    g.DrawRectangle(pen, sel.X + 1, sel.Y + 1, sel.Width - 3, sel.Height - 3);
                if (mode == "region")
                {
                    DrawHandles(g);
                    DrawSizeLabel(g);
                }
                g.SmoothingMode = SmoothingMode.Default;
            }
            DrawToolbar(g);
        }

        void DrawSelectionShadow(Graphics g, Rectangle r)
        {
            for (int i = 9; i >= 1; i--)
            {
                int d = (int)(i * 2.4f * sc);
                using (var pen = new Pen(Color.FromArgb(7, 0, 0, 0), d))
                    g.DrawRectangle(pen, r.X - d, r.Y - d, r.Width + d * 2, r.Height + d * 2);
            }
        }

        void DrawHandles(Graphics g)
        {
            if (sel.Width < 24 || sel.Height < 24) return;
            int hs = Math.Max(6, (int)(7 * sc));
            var pts = new[]
            {
                new Point(sel.Left, sel.Top),
                new Point(sel.Right - 1, sel.Top),
                new Point(sel.Left, sel.Bottom - 1),
                new Point(sel.Right - 1, sel.Bottom - 1)
            };
            using (var fill = new SolidBrush(Color.FromArgb(54, 118, 232)))
            using (var pen = new Pen(Color.White, Math.Max(1f, sc)))
            {
                foreach (var p in pts)
                {
                    var r = new Rectangle(p.X - hs / 2, p.Y - hs / 2, hs, hs);
                    g.FillRectangle(fill, r);
                    g.DrawRectangle(pen, r);
                }
            }
        }

        void DrawSizeLabel(Graphics g)
        {
            string text = sel.Width + " × " + sel.Height;
            using (var f = new Font("Segoe UI", 9f * sc, FontStyle.Regular, GraphicsUnit.Pixel))
            {
                SizeF sz = g.MeasureString(text, f);
                int pad = (int)(5 * sc);
                int labelW = (int)sz.Width + pad * 2 + (int)(2 * sc);
                int labelH = (int)sz.Height + pad;
                int x = sel.X, y = sel.Y - labelH - (int)(6 * sc);
                if (y < 0) y = Math.Min(shot.Height - labelH - 2, sel.Y + (int)(6 * sc));
                if (x + labelW > shot.Width) x = Math.Max(0, shot.Width - labelW);
                var r = new Rectangle(x, y, labelW, labelH);
                using (var path = Round(r, (int)(6 * sc)))
                using (var br = new SolidBrush(Color.FromArgb(235, 24, 25, 28)))
                using (var pen = new Pen(Color.FromArgb(60, 255, 255, 255), 1f))
                {
                    g.FillPath(br, path);
                    g.DrawPath(pen, path);
                }
                g.DrawString(text, f, Brushes.White, x + pad + (int)sc, y + pad / 2);
            }
        }

        void DrawToolbar(Graphics g)
        {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            using (var path = Round(bar, (int)(14 * sc)))
            using (var br = new SolidBrush(Color.FromArgb(238, 32, 33, 37)))
            using (var pen = new Pen(Color.FromArgb(58, 59, 65), 1f))
            {
                g.FillPath(br, path);
                g.DrawPath(pen, path);
            }

            for (int i = 0; i < 3; i++)
            {
                Rectangle r = btn[i];
                bool selected = (i == 0 && mode == "region");
                if (selected || hover == i)
                {
                    using (var path = Round(r, (int)(10 * sc)))
                    using (var br = new SolidBrush(selected ? Color.FromArgb(54, 118, 232) : Color.FromArgb(46, 255, 255, 255)))
                        g.FillPath(br, path);
                }
                DrawIcon(g, i, r);
            }

            using (var f = new Font("Segoe UI", 12f * sc, FontStyle.Regular, GraphicsUnit.Pixel))
            {
                string hint = "Esc — отмена   ·   Enter или пробел — весь экран";
                SizeF sz = g.MeasureString(hint, f);
                int x = bar.X + (bar.Width - (int)sz.Width) / 2, y = bar.Bottom + (int)(8 * sc);
                var r = new Rectangle(x - (int)(10 * sc), y - (int)(3 * sc), (int)sz.Width + (int)(20 * sc), (int)sz.Height + (int)(6 * sc));
                using (var path = Round(r, (int)(8 * sc)))
                using (var br = new SolidBrush(Color.FromArgb(170, 28, 29, 32)))
                    g.FillPath(br, path);
                using (var tb = new SolidBrush(Color.FromArgb(235, 255, 255, 255))) g.DrawString(hint, f, tb, x, y);
            }
            g.SmoothingMode = SmoothingMode.Default;
        }

        void DrawIcon(Graphics g, int i, Rectangle r)
        {
            float u = sc;
            using (var pen = new Pen(Color.White, 1.7f * u))
            {
                pen.StartCap = LineCap.Round;
                pen.EndCap = LineCap.Round;
                float cx = r.X + r.Width / 2f, cy = r.Y + r.Height / 2f;
                if (i == 0)
                {
                    pen.DashStyle = DashStyle.Dash;
                    g.DrawRectangle(pen, cx - 10 * u, cy - 8 * u, 20 * u, 16 * u);
                }
                else if (i == 1)
                {
                    g.DrawRectangle(pen, cx - 11 * u, cy - 9 * u, 22 * u, 14 * u);
                    g.DrawLine(pen, cx - 5 * u, cy + 10 * u, cx + 5 * u, cy + 10 * u);
                    g.DrawLine(pen, cx, cy + 5 * u, cx, cy + 10 * u);
                }
                else
                {
                    g.DrawLine(pen, cx - 7 * u, cy - 7 * u, cx + 7 * u, cy + 7 * u);
                    g.DrawLine(pen, cx + 7 * u, cy - 7 * u, cx - 7 * u, cy + 7 * u);
                }
            }
        }

        protected override void OnMouseMove(MouseEventArgs e)
        {
            int hb = HitButton(e.Location);
            if (hb != hover) { hover = hb; Invalidate(); }
            if (dragging)
            {
                sel = Normalize(dragStart, e.Location);
                Invalidate();
            }
            else if (mode == "full" && !bar.Contains(e.Location))
            {
                Rectangle m = MonitorRect(e.Location);
                if (m != sel) { sel = m; Invalidate(); }
            }
            Cursor = bar.Contains(e.Location) ? Cursors.Default : Cursors.Cross;
            base.OnMouseMove(e);
        }

        static Rectangle Normalize(Point a, Point b)
        {
            return Rectangle.FromLTRB(Math.Min(a.X, b.X), Math.Min(a.Y, b.Y), Math.Max(a.X, b.X), Math.Max(a.Y, b.Y));
        }

        protected override void OnMouseDown(MouseEventArgs e)
        {
            base.OnMouseDown(e);
            if (e.Button != MouseButtons.Left) return;
            int hb = HitButton(e.Location);
            if (hb == 0) { mode = "region"; sel = Rectangle.Empty; Invalidate(); return; }
            if (hb == 1) { Finish(MonitorRect(btn[1].Location)); return; }
            if (hb == 2) { Cancel(); return; }
            if (bar.Contains(e.Location)) return;
            if (mode == "full") { Finish(MonitorRect(e.Location)); return; }
            dragging = true;
            dragStart = e.Location;
            sel = Rectangle.Empty;
        }

        protected override void OnMouseUp(MouseEventArgs e)
        {
            base.OnMouseUp(e);
            if (!dragging) return;
            dragging = false;
            if (sel.Width >= 5 && sel.Height >= 5) Finish(sel);
            else { sel = Rectangle.Empty; Invalidate(); }
        }

        protected override void WndProc(ref Message m)
        {
            if (m.Msg == WM_DPICHANGED) return; // не даём форме менять размер при смене DPI
            if (m.Msg == WM_KEYDOWN)
            {
                int vk = (int)m.WParam;
                if (vk == 0x1B) { Cancel(); return; }
                if (vk == 0x0D || vk == 0x20) { Finish(MonitorRect(PointToClientSafe(Cursor.Position))); return; }
            }
            base.WndProc(ref m);
        }

        protected override void OnDeactivate(EventArgs e)
        {
            base.OnDeactivate(e);
            if (!finished && Visible && presentedAt > 0 && Environment.TickCount64 - presentedAt > 700) Cancel();
        }

        void Finish(Rectangle r)
        {
            if (finished) return;
            r.Intersect(new Rectangle(0, 0, shot.Width, shot.Height));
            if (r.Width < 1 || r.Height < 1) return;
            finished = true;
            var crop = new Bitmap(r.Width, r.Height, PixelFormat.Format32bppArgb);
            using (var g = Graphics.FromImage(crop))
                g.DrawImage(shot, new Rectangle(0, 0, r.Width, r.Height), r.X, r.Y, r.Width, r.Height, GraphicsUnit.Pixel);
            delivered = true;
            Close();
            var handler = Completed;
            if (handler != null) handler(crop);
        }

        void Cancel()
        {
            if (finished) return;
            finished = true;
            Close();
        }

        protected override void OnFormClosed(FormClosedEventArgs e)
        {
            base.OnFormClosed(e);
            shot.Dispose();
            dimmed.Dispose();
            vignette.Dispose();
            if (!delivered)
            {
                var handler = Cancelled;
                if (handler != null) handler();
            }
        }
    }
}
