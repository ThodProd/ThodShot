using System;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Shell;

namespace ThodShot
{
    /// <summary>Окно без системной рамки: тонкая шапка, один крестик, Esc закрывает.</summary>
    internal abstract class ThemedWindow : Window
    {
        const int WM_GETMINMAXINFO = 0x0024;
        const uint MONITOR_DEFAULTTONEAREST = 2;

        protected readonly Grid Body = new Grid();
        protected readonly TextBlock TitleText = new TextBlock();

        protected ThemedWindow(string title)
        {
            WindowStyle = WindowStyle.None;
            ResizeMode = ResizeMode.CanResize;
            Background = Theme.Bg;
            Foreground = Theme.Text;
            FontFamily = new FontFamily("Segoe UI Variable Text, Segoe UI");
            FontSize = 14;
            Title = title;
            UseLayoutRounding = true;
            SnapsToDevicePixels = true;
            WindowChrome.SetWindowChrome(this, new WindowChrome
            {
                CaptionHeight = 40,
                ResizeBorderThickness = new Thickness(6),
                GlassFrameThickness = new Thickness(0),
                CornerRadius = new CornerRadius(0),
                UseAeroCaptionButtons = false
            });

            var outer = new Border { BorderBrush = Theme.Line, BorderThickness = new Thickness(1), Background = Theme.Bg };
            var grid = new Grid();
            grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(40) });
            grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

            var header = new Grid();
            TitleText.Text = title;
            TitleText.Margin = new Thickness(16, 0, 0, 0);
            TitleText.VerticalAlignment = VerticalAlignment.Center;
            TitleText.FontSize = 13;
            TitleText.Foreground = Theme.Muted;
            header.Children.Add(TitleText);

            var close = new Button
            {
                Content = new System.Windows.Shapes.Path
                {
                    Data = Geometry.Parse("M0,0 L10,10 M10,0 L0,10"),
                    Stroke = Theme.Text,
                    StrokeThickness = 1.5,
                    Width = 10,
                    Height = 10
                },
                Style = Styles.Button(2),
                Width = 40,
                Height = 32,
                Padding = new Thickness(0),
                Margin = new Thickness(0, 0, 4, 0),
                HorizontalAlignment = HorizontalAlignment.Right,
                VerticalAlignment = VerticalAlignment.Center
            };
            WindowChrome.SetIsHitTestVisibleInChrome(close, true);
            close.Click += (s, e) => Close();
            header.Children.Add(close);

            Grid.SetRow(header, 0);
            Grid.SetRow(Body, 1);
            grid.Children.Add(header);
            grid.Children.Add(Body);
            outer.Child = grid;
            Content = outer;

            // Без этого окно без системной рамки при развороте уходит под панель задач.
            SourceInitialized += (s, e) =>
            {
                var source = (HwndSource)PresentationSource.FromVisual(this);
                if (source != null) source.AddHook(WindowProc);
            };

            PreviewKeyDown += (s, e) =>
            {
                if (e.Key == Key.Escape) { e.Handled = true; Close(); }
            };
        }

        IntPtr WindowProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
        {
            if (msg == WM_GETMINMAXINFO && lParam != IntPtr.Zero)
            {
                var info = (Native.MINMAXINFO)Marshal.PtrToStructure(lParam, typeof(Native.MINMAXINFO));
                IntPtr monitor = Native.MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                var mi = new Native.MONITORINFO { cbSize = Marshal.SizeOf(typeof(Native.MONITORINFO)) };
                if (monitor != IntPtr.Zero && Native.GetMonitorInfo(monitor, ref mi))
                {
                    info.ptMaxPosition.X = mi.rcWork.Left - mi.rcMonitor.Left;
                    info.ptMaxPosition.Y = mi.rcWork.Top - mi.rcMonitor.Top;
                    info.ptMaxSize.X = mi.rcWork.Right - mi.rcWork.Left;
                    info.ptMaxSize.Y = mi.rcWork.Bottom - mi.rcWork.Top;
                    Marshal.StructureToPtr(info, lParam, true);
                }
            }
            return IntPtr.Zero;
        }
    }
}
