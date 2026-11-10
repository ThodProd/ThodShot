using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing.Imaging;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Shapes;

namespace ThodShot
{
    internal enum PreviewTool { Cursor, Arrow, Diamond, Marker }

    /// <summary>Пометка на снимке. Координаты — в пикселях исходного изображения.</summary>
    internal abstract class Annotation
    {
        public abstract void Draw(DrawingContext dc);
    }

    internal abstract class TwoPointAnnotation : Annotation
    {
        public Point Start, End;
    }

    internal sealed class ArrowAnnotation : TwoPointAnnotation
    {
        static readonly Brush Red = Theme.Make("#F23B30");

        public double Thickness;
        public double Head;

        public override void Draw(DrawingContext dc)
        {
            double dx = End.X - Start.X, dy = End.Y - Start.Y;
            double len = Math.Sqrt(dx * dx + dy * dy);
            if (len < 2) return;

            double ux = dx / len, uy = dy / len;
            double head = Math.Min(Head, len * 0.62);
            double bx = End.X - ux * head, by = End.Y - uy * head;
            var pen = new Pen(Red, Thickness)
            {
                StartLineCap = PenLineCap.Round,
                EndLineCap = PenLineCap.Round
            };
            dc.DrawLine(pen, Start, new Point(bx + ux * head * 0.15, by + uy * head * 0.15));

            double px = -uy, py = ux, hw = head * 0.56;
            var geo = new StreamGeometry();
            using (var c = geo.Open())
            {
                c.BeginFigure(End, true, true);
                c.LineTo(new Point(bx + px * hw, by + py * hw), true, false);
                c.LineTo(new Point(bx - px * hw, by - py * hw), true, false);
            }
            geo.Freeze();
            dc.DrawGeometry(Red, null, geo);
        }
    }

    internal sealed class DiamondAnnotation : TwoPointAnnotation
    {
        static readonly Brush Red = Theme.Make("#F23B30");
        static readonly Brush Fill = Theme.Make("#33F23B30");

        public double Thickness;

        public override void Draw(DrawingContext dc)
        {
            double l = Math.Min(Start.X, End.X), t = Math.Min(Start.Y, End.Y);
            double w = Math.Abs(End.X - Start.X), h = Math.Abs(End.Y - Start.Y);
            if (w < 3 || h < 3) return;
            double cx = l + w / 2, cy = t + h / 2;
            var geo = new StreamGeometry();
            using (var c = geo.Open())
            {
                c.BeginFigure(new Point(cx, t), true, true);
                c.LineTo(new Point(l + w, cy), true, false);
                c.LineTo(new Point(cx, t + h), true, false);
                c.LineTo(new Point(l, cy), true, false);
            }
            geo.Freeze();
            var pen = new Pen(Red, Thickness)
            {
                LineJoin = PenLineJoin.Round,
                StartLineCap = PenLineCap.Round,
                EndLineCap = PenLineCap.Round
            };
            dc.DrawGeometry(Fill, pen, geo);
        }
    }

    internal sealed class MarkerAnnotation : Annotation
    {
        static readonly Brush Highlight = Theme.Make("#78FFD54A");

        public readonly List<Point> Points = new List<Point>();
        public double Thickness;

        public override void Draw(DrawingContext dc)
        {
            if (Points.Count == 0) return;
            if (Points.Count == 1)
            {
                dc.DrawEllipse(Highlight, null, Points[0], Thickness / 2, Thickness / 2);
                return;
            }
            var pen = new Pen(Highlight, Thickness)
            {
                StartLineCap = PenLineCap.Round,
                EndLineCap = PenLineCap.Round,
                LineJoin = PenLineJoin.Round
            };
            var geo = new StreamGeometry();
            using (var c = geo.Open())
            {
                c.BeginFigure(Points[0], false, false);
                for (int i = 1; i < Points.Count; i++) c.LineTo(Points[i], true, false);
            }
            geo.Freeze();
            dc.DrawGeometry(null, pen, geo);
        }
    }

    internal sealed class AnnotationCanvas : FrameworkElement
    {
        public readonly List<Annotation> Items = new List<Annotation>();
        public Annotation Pending;

        public AnnotationCanvas()
        {
            IsHitTestVisible = false;
            SnapsToDevicePixels = true;
        }

        public void Refresh() { InvalidateVisual(); }

        protected override void OnRender(DrawingContext dc)
        {
            base.OnRender(dc);
            foreach (var a in Items) a.Draw(dc);
            if (Pending != null) Pending.Draw(dc);
        }
    }

    /// <summary>Окно предпросмотра: масштаб, пометки, распознавание, копирование и сохранение.</summary>
    internal sealed class PreviewWindow : ThemedWindow
    {
        static readonly Brush WordFill = Theme.Make("#1A3676E8");
        static readonly Brush WordStroke = Theme.Make("#663676E8");
        static readonly Brush SelFill = Theme.Make("#593676E8");
        static readonly Brush SelStroke = Theme.Make("#3676E8");
        static readonly Brush ToolActive = Theme.Make("#2E81AAFF");
        static readonly Brush ToolbarBg = Theme.Make(Theme.Dark ? "#EE25262A" : "#EEF7F8FA");

        readonly byte[] png;
        readonly BitmapSource source;
        readonly Grid imageArea, viewport;
        readonly Grid stage;
        readonly Grid imageGrid;
        readonly AnnotationCanvas annLayer;
        readonly Canvas ocrLayer, input;
        readonly TextBlock status;
        readonly Button btnOcr, btnCopy, btnSave, btnGear;
        readonly Rectangle marquee;
        readonly ScaleTransform zoomScale = new ScaleTransform(1, 1);
        readonly TranslateTransform pan = new TranslateTransform(0, 0);
        readonly List<KeyValuePair<Button, PreviewTool>> toolButtons = new List<KeyValuePair<Button, PreviewTool>>();
        readonly double dpiScale;

        readonly List<OcrWordInfo> words = new List<OcrWordInfo>();
        readonly List<Rectangle> wordRects = new List<Rectangle>();
        readonly HashSet<int> selected = new HashSet<int>();

        double fitScale = 1, scale = 1;
        bool loaded;
        string savedPath;
        bool ocrBusy, ocrDone;
        int anchor = -1;
        bool dragWords, dragMarquee, panning, markerDrawing;
        Point marqueeStart, panStart;
        double panX0, panY0;
        PreviewTool tool = PreviewTool.Cursor;
        TwoPointAnnotation pending;
        MarkerAnnotation marker;
        Button btnUndo;
        bool suppressMenu;

        public PreviewWindow(System.Drawing.Bitmap bmp) : base("Скриншот")
        {
            using (var ms = new MemoryStream())
            {
                bmp.Save(ms, ImageFormat.Png);
                png = ms.ToArray();
            }
            bmp.Dispose();
            using (var ms = new MemoryStream(png))
            {
                var bi = new BitmapImage();
                bi.BeginInit();
                bi.CacheOption = BitmapCacheOption.OnLoad;
                bi.StreamSource = ms;
                bi.EndInit();
                bi.Freeze();
                source = bi;
            }
            TitleText.Text = "Скриншот · " + source.PixelWidth + " × " + source.PixelHeight;
            dpiScale = VisualTreeHelper.GetDpi(this).DpiScaleX;

            Body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            Body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(86) });

            // ---------- область изображения ----------
            imageArea = new Grid { Background = Theme.Surface, ClipToBounds = true };
            viewport = new Grid { Background = Theme.Surface, ClipToBounds = true };

            double iw = source.PixelWidth / dpiScale, ih = source.PixelHeight / dpiScale;
            stage = new Grid
            {
                Width = iw,
                Height = ih,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                RenderTransformOrigin = new Point(0.5, 0.5)
            };
            var tg = new TransformGroup();
            tg.Children.Add(zoomScale);
            tg.Children.Add(pan);
            stage.RenderTransform = tg;

            imageGrid = new Grid
            {
                Width = source.PixelWidth,
                Height = source.PixelHeight,
                LayoutTransform = new ScaleTransform(1.0 / dpiScale, 1.0 / dpiScale),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center
            };
            var image = new System.Windows.Controls.Image { Source = source, Stretch = Stretch.Fill };
            RenderOptions.SetBitmapScalingMode(image, BitmapScalingMode.HighQuality);

            ocrLayer = new Canvas { Background = Brushes.Transparent, Visibility = Visibility.Collapsed, IsHitTestVisible = false };
            annLayer = new AnnotationCanvas();
            input = new Canvas { Background = Brushes.Transparent, Cursor = Cursors.Arrow };
            input.MouseLeftButtonDown += OnLeftDown;
            input.MouseMove += OnMouseMove;
            input.MouseLeftButtonUp += OnLeftUp;
            input.MouseRightButtonDown += OnRightDown;
            input.ContextMenuOpening += (s, e) =>
            {
                if (suppressMenu) { suppressMenu = false; e.Handled = true; }
                else if (ocrLayer.Visibility != Visibility.Visible) e.Handled = true;
            };

            marquee = new Rectangle
            {
                Stroke = SelStroke,
                StrokeThickness = 1.5,
                StrokeDashArray = new DoubleCollection { 4, 3 },
                Fill = Theme.Make("#223676E8"),
                Visibility = Visibility.Collapsed,
                IsHitTestVisible = false
            };

            imageGrid.Children.Add(image);
            imageGrid.Children.Add(ocrLayer);
            imageGrid.Children.Add(annLayer);
            imageGrid.Children.Add(input);
            stage.Children.Add(imageGrid);
            viewport.Children.Add(stage);

            viewport.SizeChanged += (s, e) => { if (loaded) OnViewportResized(); };

            imageArea.Children.Add(viewport);
            imageArea.Children.Add(BuildToolbar());
            imageArea.PreviewMouseWheel += OnWheel;
            Grid.SetRow(imageArea, 0);
            Body.Children.Add(imageArea);

            // ---------- нижняя панель ----------
            var bottom = new Border
            {
                BorderBrush = Theme.Line,
                BorderThickness = new Thickness(0, 1, 0, 0),
                Background = Theme.Bg,
                Padding = new Thickness(14, 8, 14, 10)
            };
            var bar = new Grid();
            bar.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            bar.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });

            status = new TextBlock
            {
                Foreground = Theme.Muted,
                FontSize = 12.5,
                VerticalAlignment = VerticalAlignment.Center,
                TextTrimming = TextTrimming.CharacterEllipsis,
                Margin = new Thickness(2, 0, 2, 7)
            };
            bar.Children.Add(status);

            var actions = new Grid();
            actions.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            actions.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

            btnOcr = MakeButton("Распознать текст", 0, OnOcrClick);
            btnOcr.ToolTip = "Найти текст на снимке (Ctrl+C — копировать выделенное)";
            btnOcr.HorizontalAlignment = HorizontalAlignment.Left;
            actions.Children.Add(btnOcr);

            var right = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
            btnCopy = MakeButton("Копировать", 0, CopyImageWithStatus);
            btnCopy.ToolTip = "Скопировать изображение с пометками (Ctrl+C)";
            btnSave = MakeButton("Сохранить", 1, DoSave);
            btnSave.ToolTip = "Сохранить с пометками (Ctrl+S)";
            btnGear = MakeButton("\uE713", 2, () => { if (Resident.Instance != null) Resident.Instance.ShowSettings(); });
            btnGear.FontFamily = new FontFamily("Segoe MDL2 Assets");
            btnGear.FontSize = 16;
            btnGear.Padding = new Thickness(10, 6, 10, 6);
            btnGear.ToolTip = "Настройки";
            foreach (var b in new[] { btnCopy, btnSave, btnGear })
            {
                b.Margin = new Thickness(8, 0, 0, 0);
                right.Children.Add(b);
            }
            Grid.SetColumn(right, 1);
            actions.Children.Add(right);
            Grid.SetRow(actions, 1);
            bar.Children.Add(actions);

            bottom.Child = bar;
            Grid.SetRow(bottom, 1);
            Body.Children.Add(bottom);

            // размер окна: по размеру снимка, но не больше 85% рабочей области
            var wa = SystemParameters.WorkArea;
            MinWidth = 600;
            MinHeight = 380;
            Width = Math.Max(MinWidth, Math.Min(iw + 26, wa.Width * 0.85));
            Height = Math.Max(MinHeight, Math.Min(ih + 24 + 40 + 86 + 2, wa.Height * 0.85));
            WindowStartupLocation = WindowStartupLocation.CenterScreen;

            Loaded += (s, e) =>
            {
                loaded = true;
                UpdateScale();
                ResetFit();
            };
            DpiChanged += (s, e) => { UpdateScale(); ResetFit(); };

            PreviewKeyDown += OnKey;
            SetStatus("Колесо мыши — масштаб · Перетаскивание — сдвиг · Ctrl+Z — отменить пометку");
        }

        // ---------- панель инструментов ----------

        FrameworkElement BuildToolbar()
        {
            var card = new Border
            {
                Background = ToolbarBg,
                BorderBrush = Theme.Line,
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(5),
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Top,
                Margin = new Thickness(0, 10, 0, 0)
            };
            Panel.SetZIndex(card, 10);
            var row = new StackPanel { Orientation = Orientation.Horizontal };

            row.Children.Add(MakeToolButton(PreviewTool.Cursor, "Курсор и сдвиг",
                Geometry.Parse("M 4,3 L 4,19 L 8.8,14.3 L 11.7,20 L 14.6,18.6 L 11.7,13 L 17.5,13 Z"), true));
            row.Children.Add(MakeToolButton(PreviewTool.Arrow, "Красная стрелка",
                Geometry.Parse("M 3,17 L 16.5,3.5 M 8.5,3.5 L 16.5,3.5 L 16.5,11.5"), false));
            row.Children.Add(MakeToolButton(PreviewTool.Diamond, "Ромб",
                Geometry.Parse("M 10,2.5 L 17.5,10 L 10,17.5 L 2.5,10 Z"), false));
            row.Children.Add(MakeToolButton(PreviewTool.Marker, "Маркер",
                Geometry.Parse("M 4,15.5 L 12.8,6.7 L 15.3,9.2 L 6.5,18 L 3,19 Z M 13.7,5.8 L 15.2,4.3 L 17.7,6.8 L 16.2,8.3"), false));

            row.Children.Add(new Border
            {
                Width = 1,
                Height = 20,
                Background = Theme.Line,
                Margin = new Thickness(5, 0, 5, 0),
                VerticalAlignment = VerticalAlignment.Center
            });

            btnUndo = MakeIconButton("M 8,5 L 3.5,9.5 L 8,14 M 3.5,9.5 L 12,9.5 A 4.6,4.6 0 0 1 12,18.7 L 8.5,18.7",
                "Отменить (Ctrl+Z)", Undo);
            btnUndo.IsEnabled = false;
            row.Children.Add(btnUndo);

            card.Child = row;
            return card;
        }

        Button MakeToolButton(PreviewTool t, string tip, Geometry icon, bool filled)
        {
            var b = MakeIconButton(icon, tip, () => SetTool(t), filled);
            toolButtons.Add(new KeyValuePair<Button, PreviewTool>(b, t));
            if (t == tool) b.Background = ToolActive;
            return b;
        }

        Button MakeIconButton(string data, string tip, Action click)
        {
            return MakeIconButton(Geometry.Parse(data), tip, click, false);
        }

        Button MakeIconButton(Geometry icon, string tip, Action click, bool filled = false)
        {
            var path = new System.Windows.Shapes.Path
            {
                Data = icon,
                Stroke = Theme.Text,
                StrokeThickness = 1.55,
                StrokeLineJoin = PenLineJoin.Round,
                StrokeStartLineCap = PenLineCap.Round,
                StrokeEndLineCap = PenLineCap.Round,
                Fill = filled ? Theme.Text : null,
                Width = 18,
                Height = 18,
                Stretch = Stretch.Uniform
            };
            var b = new Button
            {
                Content = path,
                Style = Styles.Button(2),
                Width = 34,
                Height = 34,
                Padding = new Thickness(8),
                Margin = new Thickness(1, 0, 1, 0),
                Background = Brushes.Transparent,
                ToolTip = tip
            };
            b.Click += (s, e) => click();
            ToolTipService.SetInitialShowDelay(b, 500);
            return b;
        }

        Button MakeButton(string text, int kind, Action click)
        {
            var b = new Button { Content = text, Style = Styles.Button(kind) };
            b.Click += (s, e) => click();
            return b;
        }

        void SetTool(PreviewTool t)
        {
            CancelPending(false);
            tool = t;
            foreach (var pair in toolButtons)
                pair.Key.Background = pair.Value == tool ? ToolActive : Brushes.Transparent;
            input.Cursor = tool == PreviewTool.Cursor ? Cursors.Arrow : Cursors.Cross;
        }

        // ---------- масштаб и сдвиг ----------

        void UpdateScale()
        {
            var dpi = VisualTreeHelper.GetDpi(this);
            imageGrid.LayoutTransform = new ScaleTransform(1.0 / dpi.DpiScaleX, 1.0 / dpi.DpiScaleY);
            stage.Width = source.PixelWidth / dpi.DpiScaleX;
            stage.Height = source.PixelHeight / dpi.DpiScaleY;
        }

        void ResetFit()
        {
            if (viewport.ActualWidth < 8 || viewport.ActualHeight < 8) return;
            fitScale = Math.Min(1.0, Math.Min(viewport.ActualWidth / stage.Width, viewport.ActualHeight / stage.Height));
            fitScale = Math.Max(0.02, fitScale);
            scale = fitScale;
            pan.X = pan.Y = 0;
            ApplyZoom();
        }

        void OnViewportResized()
        {
            double oldFit = fitScale;
            if (viewport.ActualWidth < 8 || viewport.ActualHeight < 8) return;
            fitScale = Math.Min(1.0, Math.Min(viewport.ActualWidth / stage.Width, viewport.ActualHeight / stage.Height));
            fitScale = Math.Max(0.02, fitScale);
            if (scale <= oldFit + 0.001) { scale = fitScale; pan.X = pan.Y = 0; }
            else if (scale < fitScale) scale = fitScale;
            ClampPan();
            ApplyZoom();
        }

        void OnWheel(object sender, MouseWheelEventArgs e)
        {
            if (e.Delta == 0) return;
            double factor = e.Delta > 0 ? 1.16 : 1 / 1.16;
            ZoomAt(e.GetPosition(viewport), scale * factor);
            e.Handled = true;
        }

        void ZoomAt(Point viewportPoint, double target)
        {
            double ns = Math.Max(fitScale, Math.Min(8.0, target));
            if (Math.Abs(ns - scale) < 0.0001) return;
            Point p = viewport.TranslatePoint(viewportPoint, stage);
            double dx = p.X - stage.Width / 2;
            double dy = p.Y - stage.Height / 2;
            pan.X += (scale - ns) * dx;
            pan.Y += (scale - ns) * dy;
            scale = ns;
            if (Math.Abs(scale - fitScale) < 0.0001) { pan.X = pan.Y = 0; }
            ClampPan();
            ApplyZoom();
        }

        void ClampPan()
        {
            double vw = viewport.ActualWidth, vh = viewport.ActualHeight;
            double sw = stage.Width * scale, sh = stage.Height * scale;
            double maxX = Math.Max(0, (sw - vw) / 2);
            double maxY = Math.Max(0, (sh - vh) / 2);
            pan.X = Math.Max(-maxX, Math.Min(maxX, pan.X));
            pan.Y = Math.Max(-maxY, Math.Min(maxY, pan.Y));
        }

        void ApplyZoom()
        {
            zoomScale.ScaleX = zoomScale.ScaleY = scale;
            UpdateCursor(null);
        }

        void SetStatus(string text, bool error = false)
        {
            status.Text = text;
            status.Foreground = error ? Theme.Danger : Theme.Muted;
        }

        // ---------- мышь ----------

        void OnLeftDown(object sender, MouseButtonEventArgs e)
        {
            Focus();
            Point p = e.GetPosition(input);

            if (tool == PreviewTool.Cursor)
            {
                if (ocrLayer.Visibility == Visibility.Visible && words.Count > 0)
                {
                    int idx = HitWord(p, 6);
                    if (idx >= 0)
                    {
                        input.CaptureMouse();
                        dragWords = true;
                        anchor = idx;
                        Select(new[] { idx });
                        e.Handled = true;
                        return;
                    }
                    if (scale <= fitScale + 0.001)
                    {
                        input.CaptureMouse();
                        dragMarquee = true;
                        marqueeStart = p;
                        Select(new int[0]);
                        marquee.Width = 0;
                        marquee.Height = 0;
                        Canvas.SetLeft(marquee, p.X);
                        Canvas.SetTop(marquee, p.Y);
                        marquee.Visibility = Visibility.Visible;
                        e.Handled = true;
                        return;
                    }
                }
                if (scale > fitScale + 0.001)
                {
                    input.CaptureMouse();
                    panning = true;
                    panStart = e.GetPosition(viewport);
                    panX0 = pan.X;
                    panY0 = pan.Y;
                    input.Cursor = Cursors.SizeAll;
                }
                e.Handled = true;
                return;
            }

            if (tool == PreviewTool.Marker)
            {
                marker = new MarkerAnnotation { Thickness = Math.Max(11, Math.Min(source.PixelWidth, source.PixelHeight) * 0.011) };
                marker.Points.Add(p);
                annLayer.Items.Add(marker);
                markerDrawing = true;
                input.CaptureMouse();
                annLayer.Refresh();
                SetStatus("Маркер: ведите мышью, отпустите кнопку");
                e.Handled = true;
                return;
            }

            if (pending == null)
            {
                if (tool == PreviewTool.Arrow)
                    pending = new ArrowAnnotation
                    {
                        Start = p,
                        End = p,
                        Thickness = Math.Max(3, Math.Min(source.PixelWidth, source.PixelHeight) * 0.0036),
                        Head = Math.Max(14, Math.Min(source.PixelWidth, source.PixelHeight) * 0.019)
                    };
                else
                    pending = new DiamondAnnotation
                    {
                        Start = p,
                        End = p,
                        Thickness = Math.Max(3, Math.Min(source.PixelWidth, source.PixelHeight) * 0.0036)
                    };
                annLayer.Pending = pending;
                annLayer.Refresh();
                UpdateUndo();
                SetStatus("Второй клик — зафиксировать · правая кнопка — отменить");
            }
            else
            {
                pending.End = p;
                CommitPending();
            }
            e.Handled = true;
        }

        void OnMouseMove(object sender, MouseEventArgs e)
        {
            Point p = e.GetPosition(input);

            if (panning)
            {
                Point now = e.GetPosition(viewport);
                pan.X = panX0 + (now.X - panStart.X);
                pan.Y = panY0 + (now.Y - panStart.Y);
                ClampPan();
                zoomScale.ScaleX = zoomScale.ScaleY = scale;
                return;
            }

            if (dragWords)
            {
                int j = NearestWord(p);
                if (j >= 0) Select(Enumerable.Range(Math.Min(anchor, j), Math.Abs(anchor - j) + 1));
                return;
            }
            if (dragMarquee)
            {
                double x = Math.Min(marqueeStart.X, p.X), y = Math.Min(marqueeStart.Y, p.Y);
                double w = Math.Abs(p.X - marqueeStart.X), h = Math.Abs(p.Y - marqueeStart.Y);
                Canvas.SetLeft(marquee, x);
                Canvas.SetTop(marquee, y);
                marquee.Width = w;
                marquee.Height = h;
                Select(Enumerable.Range(0, words.Count).Where(i =>
                {
                    var wd = words[i];
                    return wd.X < x + w && wd.X + wd.W > x && wd.Y < y + h && wd.Y + wd.H > y;
                }));
                return;
            }
            if (markerDrawing && marker != null)
            {
                Point last = marker.Points[marker.Points.Count - 1];
                double dx = p.X - last.X, dy = p.Y - last.Y;
                if (dx * dx + dy * dy >= 4)
                {
                    marker.Points.Add(p);
                    annLayer.Refresh();
                }
                return;
            }
            if (pending != null)
            {
                pending.End = p;
                annLayer.Refresh();
                return;
            }

            UpdateCursor(p);
        }

        void OnLeftUp(object sender, MouseButtonEventArgs e)
        {
            if (panning)
            {
                panning = false;
                input.ReleaseMouseCapture();
                UpdateCursor(e.GetPosition(input));
            }
            if (markerDrawing)
            {
                markerDrawing = false;
                input.ReleaseMouseCapture();
                marker = null;
                AnnotationsChanged();
                SetStatus("Пометка добавлена · Ctrl+Z — отменить");
            }
            if (dragWords || dragMarquee)
            {
                dragWords = false;
                dragMarquee = false;
                marquee.Visibility = Visibility.Collapsed;
                input.ReleaseMouseCapture();
                if (selected.Count > 0) SetStatus("Выделено слов: " + selected.Count + " · Ctrl+C — копировать текст");
            }
        }

        void OnRightDown(object sender, MouseButtonEventArgs e)
        {
            if (markerDrawing && marker != null)
            {
                annLayer.Items.Remove(marker);
                marker = null;
                markerDrawing = false;
                input.ReleaseMouseCapture();
                annLayer.Refresh();
                UpdateUndo();
                suppressMenu = true;
                e.Handled = true;
                return;
            }
            if (pending != null)
            {
                CancelPending(true);
                suppressMenu = true;
                e.Handled = true;
            }
        }

        void CancelPending(bool report)
        {
            if (pending == null) return;
            pending = null;
            annLayer.Pending = null;
            annLayer.Refresh();
            UpdateUndo();
            if (report) SetStatus("Фигура отменена");
        }

        void CommitPending()
        {
            if (pending == null) return;
            var a = pending;
            pending = null;
            annLayer.Pending = null;

            bool tiny = false;
            var arrow = a as ArrowAnnotation;
            var diamond = a as DiamondAnnotation;
            if (arrow != null)
            {
                double dx = arrow.End.X - arrow.Start.X, dy = arrow.End.Y - arrow.Start.Y;
                tiny = dx * dx + dy * dy < 25;
            }
            else if (diamond != null)
                tiny = Math.Abs(diamond.End.X - diamond.Start.X) < 4 || Math.Abs(diamond.End.Y - diamond.Start.Y) < 4;

            if (!tiny)
            {
                annLayer.Items.Add(a);
                SetStatus("Пометка добавлена · Ctrl+Z — отменить");
            }
            annLayer.Refresh();
            AnnotationsChanged();
        }

        void AnnotationsChanged()
        {
            UpdateUndo();
            if (savedPath != null) SetStatus("Изображение изменено · Ctrl+S — обновить файл");
        }

        void UpdateUndo()
        {
            if (btnUndo != null) btnUndo.IsEnabled = annLayer.Items.Count > 0 || pending != null;
        }

        void Undo()
        {
            if (pending != null) { CancelPending(true); return; }
            if (annLayer.Items.Count == 0) return;
            annLayer.Items.RemoveAt(annLayer.Items.Count - 1);
            annLayer.Refresh();
            AnnotationsChanged();
            SetStatus("Последняя пометка отменена");
        }

        void UpdateCursor(Point? at)
        {
            if (tool != PreviewTool.Cursor) { input.Cursor = Cursors.Cross; return; }
            if (panning) { input.Cursor = Cursors.SizeAll; return; }
            if (at.HasValue && ocrLayer.Visibility == Visibility.Visible && words.Count > 0 && HitWord(at.Value, 6) >= 0)
            {
                input.Cursor = Cursors.IBeam;
                return;
            }
            if (at.HasValue && scale > fitScale + 0.001) { input.Cursor = Cursors.SizeAll; return; }
            input.Cursor = Cursors.Arrow;
        }

        // ---------- клавиши ----------

        void OnKey(object sender, KeyEventArgs e)
        {
            bool ctrl = (Keyboard.Modifiers & ModifierKeys.Control) != 0;
            if (!ctrl) return;
            if (e.Key == Key.C)
            {
                if (selected.Count > 0 && ocrLayer.Visibility == Visibility.Visible) CopyText(); else CopyImageWithStatus();
                e.Handled = true;
            }
            else if (e.Key == Key.S) { DoSave(); e.Handled = true; }
            else if (e.Key == Key.A && ocrLayer.Visibility == Visibility.Visible) { SelectAll(); e.Handled = true; }
            else if (e.Key == Key.Z) { Undo(); e.Handled = true; }
        }

        // ---------- копирование / сохранение ----------

        static void WithClipboard(Action a)
        {
            for (int i = 0; i < 6; i++)
            {
                try { a(); return; }
                catch (COMException) { Thread.Sleep(80); }
            }
            a();
        }

        BitmapSource Compose()
        {
            if (annLayer.Items.Count == 0) return source;
            var dv = new DrawingVisual();
            using (var dc = dv.RenderOpen())
            {
                dc.DrawImage(source, new Rect(0, 0, source.PixelWidth, source.PixelHeight));
                foreach (var a in annLayer.Items) a.Draw(dc);
            }
            var rtb = new RenderTargetBitmap(source.PixelWidth, source.PixelHeight, 96, 96, PixelFormats.Pbgra32);
            rtb.Render(dv);
            rtb.Freeze();
            return rtb;
        }

        void CopyImage(BitmapSource img)
        {
            WithClipboard(() => Clipboard.SetImage(img));
        }

        void CopyImageWithStatus()
        {
            try
            {
                var img = Compose();
                CopyImage(img);
                SetStatus(annLayer.Items.Count > 0 ? "Изображение с пометками скопировано" : "Изображение скопировано в буфер обмена");
            }
            catch (Exception ex) { Log.Write(ex.ToString()); SetStatus("Не удалось скопировать: " + ex.Message, true); }
        }

        void DoSave()
        {
            try
            {
                var s = AppSettings.Current;
                string ext = savedPath != null
                    ? System.IO.Path.GetExtension(savedPath).TrimStart('.').ToLowerInvariant()
                    : (s.Format == "jpg" ? "jpg" : "png");
                if (ext != "jpg" && ext != "png") ext = "png";
                if (savedPath == null || !File.Exists(savedPath))
                {
                    string folder = s.EffectiveFolder;
                    Directory.CreateDirectory(folder);
                    string baseName = "Скриншот " + DateTime.Now.ToString("yyyy-MM-dd HH-mm-ss");
                    string path = System.IO.Path.Combine(folder, baseName + "." + ext);
                    int n = 1;
                    while (File.Exists(path)) path = System.IO.Path.Combine(folder, baseName + " (" + (n++) + ")." + ext);
                    savedPath = path;
                }

                var img = Compose();
                if (ext == "png")
                {
                    if (annLayer.Items.Count == 0) File.WriteAllBytes(savedPath, png);
                    else
                    {
                        var enc = new PngBitmapEncoder();
                        enc.Frames.Add(BitmapFrame.Create(img));
                        using (var fs = File.Create(savedPath)) enc.Save(fs);
                    }
                }
                else
                {
                    var enc = new JpegBitmapEncoder { QualityLevel = 92 };
                    enc.Frames.Add(BitmapFrame.Create(img));
                    using (var fs = File.Create(savedPath)) enc.Save(fs);
                }

                CopyImage(img);
                SetStatus("Сохранено: " + System.IO.Path.GetFileName(savedPath) + " · картинка в буфере");
                if (s.OpenFolderAfterSave)
                    Process.Start(new ProcessStartInfo("explorer.exe", "/select,\"" + savedPath + "\"") { UseShellExecute = true });
            }
            catch (Exception ex)
            {
                Log.Write(ex.ToString());
                SetStatus("Не удалось сохранить: " + ex.Message, true);
            }
        }

        // ---------- распознавание ----------

        async void OnOcrClick()
        {
            if (ocrBusy) return;
            if (ocrDone)
            {
                bool show = ocrLayer.Visibility != Visibility.Visible;
                ocrLayer.Visibility = show ? Visibility.Visible : Visibility.Collapsed;
                input.ContextMenu = show ? BuildContextMenu() : null;
                btnOcr.Content = show ? "Скрыть текст" : "Показать текст";
                if (!show) Select(new int[0]);
                SetStatus(show ? "Выделите текст мышью · Ctrl+C — копировать" : "Подсветка текста скрыта");
                UpdateCursor(null);
                return;
            }

            ocrBusy = true;
            btnOcr.IsEnabled = false;
            btnOcr.Content = "Распознаю…";
            SetStatus("Распознавание текста…");
            OcrOutcome res;
            try { res = await OcrService.RecognizeAsync(png, AppSettings.Current.OcrLanguage); }
            catch (Exception ex)
            {
                Log.Write(ex.ToString());
                res = new OcrOutcome { Error = ex.Message };
            }
            ocrBusy = false;
            btnOcr.IsEnabled = true;
            btnOcr.Content = "Распознать текст";

            if (res.NoLanguage)
            {
                SetStatus("Не установлен языковой пакет распознавания Windows", true);
                var answer = MessageBox.Show(this,
                    "Для распознавания нужен языковой пакет Windows с поддержкой текста (русский или английский).\n\nОткрыть настройки языка Windows?",
                    "ThodShot", MessageBoxButton.YesNo, MessageBoxImage.Information);
                if (answer == MessageBoxResult.Yes)
                {
                    try { Process.Start(new ProcessStartInfo("ms-settings:regionlanguage") { UseShellExecute = true }); } catch { }
                }
                return;
            }
            if (res.Error != null) { SetStatus("Ошибка распознавания: " + res.Error, true); return; }
            if (res.Words.Count == 0) { SetStatus("Текст на снимке не найден"); return; }

            BuildOcrLayer(res.Words);
            ocrDone = true;
            ocrLayer.Visibility = Visibility.Visible;
            input.ContextMenu = BuildContextMenu();
            btnOcr.Content = "Скрыть текст";
            SetStatus("Найдено слов: " + words.Count + ". Выделите текст мышью, Ctrl+C — копировать");
        }

        void BuildOcrLayer(List<OcrWordInfo> found)
        {
            ocrLayer.Children.Clear();
            words.Clear();
            wordRects.Clear();
            selected.Clear();
            words.AddRange(found);
            foreach (var w in words)
            {
                var r = new Rectangle
                {
                    Width = w.W + 2,
                    Height = w.H + 2,
                    Fill = WordFill,
                    Stroke = WordStroke,
                    StrokeThickness = 0.8,
                    RadiusX = 2,
                    RadiusY = 2,
                    IsHitTestVisible = false
                };
                Canvas.SetLeft(r, w.X - 1);
                Canvas.SetTop(r, w.Y - 1);
                ocrLayer.Children.Add(r);
                wordRects.Add(r);
            }
            ocrLayer.Children.Add(marquee);
        }

        int HitWord(Point p, double tol)
        {
            for (int i = 0; i < words.Count; i++)
            {
                var w = words[i];
                if (p.X >= w.X - tol && p.X <= w.X + w.W + tol && p.Y >= w.Y - tol && p.Y <= w.Y + w.H + tol) return i;
            }
            return -1;
        }

        int NearestWord(Point p)
        {
            int best = -1;
            double bestD = double.MaxValue;
            for (int i = 0; i < words.Count; i++)
            {
                var w = words[i];
                double dx = Math.Max(Math.Max(w.X - p.X, 0), p.X - (w.X + w.W));
                double dy = Math.Max(Math.Max(w.Y - p.Y, 0), p.Y - (w.Y + w.H));
                double d = dx * dx + dy * dy;
                if (d < bestD) { bestD = d; best = i; }
            }
            return best;
        }

        void Select(IEnumerable<int> indexes)
        {
            selected.Clear();
            foreach (int i in indexes) selected.Add(i);
            for (int i = 0; i < wordRects.Count; i++)
            {
                bool on = selected.Contains(i);
                wordRects[i].Fill = on ? SelFill : WordFill;
                wordRects[i].Stroke = on ? SelStroke : WordStroke;
            }
        }

        void SelectAll() { Select(Enumerable.Range(0, words.Count)); }

        string BuildText(IEnumerable<int> indexes)
        {
            var sb = new StringBuilder();
            int prevLine = -1;
            foreach (int i in indexes.OrderBy(x => x))
            {
                var w = words[i];
                if (prevLine >= 0) sb.Append(w.Line == prevLine ? " " : "\n");
                sb.Append(w.Text);
                prevLine = w.Line;
            }
            return sb.ToString();
        }

        void CopyText()
        {
            if (words.Count == 0) return;
            try
            {
                bool all = selected.Count == 0;
                string text = BuildText(all ? Enumerable.Range(0, words.Count) : (IEnumerable<int>)selected);
                WithClipboard(() => Clipboard.SetText(text));
                SetStatus(all ? "Весь текст скопирован" : "Выделенный текст скопирован");
            }
            catch (Exception ex) { Log.Write(ex.ToString()); SetStatus("Не удалось скопировать текст: " + ex.Message, true); }
        }

        ContextMenu BuildContextMenu()
        {
            var menu = new ContextMenu();
            var copy = new MenuItem { Header = "Копировать текст" };
            copy.Click += (s, e) => CopyText();
            var all = new MenuItem { Header = "Выделить всё" };
            all.Click += (s, e) => SelectAll();
            menu.Items.Add(copy);
            menu.Items.Add(all);
            return menu;
        }
    }
}
