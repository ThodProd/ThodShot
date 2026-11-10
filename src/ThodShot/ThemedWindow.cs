using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shell;

namespace ThodShot
{
    /// <summary>Окно без системной рамки: тонкая шапка, один крестик, Esc закрывает.</summary>
    internal abstract class ThemedWindow : Window
    {
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

            PreviewKeyDown += (s, e) =>
            {
                if (e.Key == Key.Escape) { e.Handled = true; Close(); }
            };
        }
    }
}
