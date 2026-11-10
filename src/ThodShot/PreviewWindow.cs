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
    /// <summary>Небольшое окно предпросмотра: Распознать, Копировать, Сохранить.</summary>
    internal sealed class PreviewWindow : ThemedWindow
    {
        static readonly Brush WordFill = Theme.Make("#1A3676E8");
        static readonly Brush WordStroke = Theme.Make("#663676E8");
        static readonly Brush SelFill = Theme.Make("#593676E8");
        static readonly Brush SelStroke = Theme.Make("#3676E8");

        readonly byte[] png;
        readonly BitmapSource source;
        readonly Grid imageGrid;
        readonly Canvas ocrLayer;
        readonly TextBlock status;
        readonly Button btnOcr, btnCopyText;
        readonly Rectangle marquee;

        readonly List<OcrWordInfo> words = new List<OcrWordInfo>();
        readonly List<Rectangle> wordRects = new List<Rectangle>();
        readonly HashSet<int> selected = new HashSet<int>();

        string savedPath;
        bool ocrBusy, ocrDone;
        int anchor = -1;
        bool dragWords, dragMarquee;
        Point marqueeStart;

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

            Body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            Body.RowDefinitions.Add(new RowDefinition { Height = new GridLength(60) });

            // область изображения
            var viewbox = new Viewbox
            {
                Stretch = Stretch.Uniform,
                StretchDirection = StretchDirection.DownOnly,
                Margin = new Thickness(12)
            };
            imageGrid = new Grid { Width = source.PixelWidth, Height = source.PixelHeight };
            var image = new System.Windows.Controls.Image { Source = source, Stretch = Stretch.Fill };
            RenderOptions.SetBitmapScalingMode(image, BitmapScalingMode.HighQuality);
            ocrLayer = new Canvas { Background = Brushes.Transparent, Visibility = Visibility.Collapsed, Cursor = Cursors.IBeam };
            marquee = new Rectangle
            {
                Stroke = SelStroke,
                StrokeThickness = 1.5,
                StrokeDashArray = new DoubleCollection { 4, 3 },
                Fill = Theme.Make("#223676E8"),
                Visibility = Visibility.Collapsed,
                IsHitTestVisible = false
            };
            ocrLayer.MouseLeftButtonDown += OnLayerDown;
            ocrLayer.MouseMove += OnLayerMove;
            ocrLayer.MouseLeftButtonUp += OnLayerUp;
            ocrLayer.ContextMenu = BuildContextMenu();
            imageGrid.Children.Add(image);
            imageGrid.Children.Add(ocrLayer);
            viewbox.Child = imageGrid;

            var imageArea = new Border { Background = Theme.Surface, Child = viewbox };
            Grid.SetRow(imageArea, 0);
            Body.Children.Add(imageArea);

            // нижняя панель
            var bottom = new Border
            {
                BorderBrush = Theme.Line,
                BorderThickness = new Thickness(0, 1, 0, 0),
                Background = Theme.Bg,
                Padding = new Thickness(16, 0, 12, 0)
            };
            var bar = new Grid();
            bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            bar.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

            status = new TextBlock
            {
                Foreground = Theme.Muted,
                FontSize = 13,
                VerticalAlignment = VerticalAlignment.Center,
                TextTrimming = TextTrimming.CharacterEllipsis,
                Margin = new Thickness(0, 0, 12, 0)
            };
            bar.Children.Add(status);

            var buttons = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
            btnOcr = MakeButton("Распознать", 1, OnOcrClick);
            btnCopyText = MakeButton("Копировать текст", 0, CopyText);
            btnCopyText.Visibility = Visibility.Collapsed;
            var btnCopy = MakeButton("Копировать", 0, CopyImageWithStatus);
            var btnSave = MakeButton("Сохранить", 0, DoSave);
            var btnGear = MakeButton("\uE713", 2, () => { if (Resident.Instance != null) Resident.Instance.ShowSettings(); });
            btnGear.FontFamily = new FontFamily("Segoe MDL2 Assets");
            btnGear.FontSize = 16;
            btnGear.Padding = new Thickness(10, 6, 10, 6);
            btnGear.ToolTip = "Настройки";
            foreach (var b in new[] { btnOcr, btnCopyText, btnCopy, btnSave, btnGear })
            {
                b.Margin = new Thickness(8, 0, 0, 0);
                buttons.Children.Add(b);
            }
            Grid.SetColumn(buttons, 1);
            bar.Children.Add(buttons);
            bottom.Child = bar;
            Grid.SetRow(bottom, 1);
            Body.Children.Add(bottom);

            // размер окна: по размеру снимка, но не больше 85% рабочей области
            var wa = SystemParameters.WorkArea;
            double ds = VisualTreeHelper.GetDpi(this).DpiScaleX;
            double iw = source.PixelWidth / ds, ih = source.PixelHeight / ds;
            MinWidth = 560;
            MinHeight = 340;
            Width = Math.Max(MinWidth, Math.Min(iw + 26, wa.Width * 0.85));
            Height = Math.Max(MinHeight, Math.Min(ih + 24 + 40 + 60 + 2, wa.Height * 0.85));
            WindowStartupLocation = WindowStartupLocation.CenterScreen;

            Loaded += (s, e) => UpdateScale();
            DpiChanged += (s, e) => UpdateScale();

            PreviewKeyDown += OnKey;
            SetStatus("Esc — закрыть · Ctrl+S — сохранить · Ctrl+C — копировать");
        }

        void UpdateScale()
        {
            var dpi = VisualTreeHelper.GetDpi(this);
            imageGrid.LayoutTransform = new ScaleTransform(1.0 / dpi.DpiScaleX, 1.0 / dpi.DpiScaleY);
        }

        Button MakeButton(string text, int kind, Action click)
        {
            var b = new Button { Content = text, Style = Styles.Button(kind) };
            b.Click += (s, e) => click();
            return b;
        }

        void SetStatus(string text, bool error = false)
        {
            status.Text = text;
            status.Foreground = error ? Theme.Danger : Theme.Muted;
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

        void OnKey(object sender, KeyEventArgs e)
        {
            bool ctrl = (Keyboard.Modifiers & ModifierKeys.Control) != 0;
            if (!ctrl) return;
            if (e.Key == Key.C)
            {
                if (selected.Count > 0) CopyText(); else CopyImageWithStatus();
                e.Handled = true;
            }
            else if (e.Key == Key.S) { DoSave(); e.Handled = true; }
            else if (e.Key == Key.A && ocrLayer.Visibility == Visibility.Visible) { SelectAll(); e.Handled = true; }
        }

        // ---------- Копирование / сохранение ----------

        static void WithClipboard(Action a)
        {
            for (int i = 0; i < 6; i++)
            {
                try { a(); return; }
                catch (COMException) { Thread.Sleep(80); }
            }
            a();
        }

        void CopyImage()
        {
            WithClipboard(() => Clipboard.SetImage(source));
        }

        void CopyImageWithStatus()
        {
            try { CopyImage(); SetStatus("Изображение скопировано в буфер обмена"); }
            catch (Exception ex) { Log.Write(ex.ToString()); SetStatus("Не удалось скопировать: " + ex.Message, true); }
        }

        void DoSave()
        {
            try
            {
                var s = AppSettings.Current;
                if (savedPath == null || !File.Exists(savedPath))
                {
                    string folder = s.EffectiveFolder;
                    Directory.CreateDirectory(folder);
                    string ext = s.Format == "jpg" ? "jpg" : "png";
                    string baseName = "Скриншот " + DateTime.Now.ToString("yyyy-MM-dd HH-mm-ss");
                    string path = System.IO.Path.Combine(folder, baseName + "." + ext);
                    int n = 1;
                    while (File.Exists(path)) path = System.IO.Path.Combine(folder, baseName + " (" + (n++) + ")." + ext);

                    if (ext == "png") File.WriteAllBytes(path, png);
                    else
                    {
                        var enc = new JpegBitmapEncoder { QualityLevel = 92 };
                        enc.Frames.Add(BitmapFrame.Create(source));
                        using (var fs = File.Create(path)) enc.Save(fs);
                    }
                    savedPath = path;
                }
                CopyImage();
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

        // ---------- Распознавание ----------

        async void OnOcrClick()
        {
            if (ocrBusy) return;
            if (ocrDone)
            {
                bool show = ocrLayer.Visibility != Visibility.Visible;
                ocrLayer.Visibility = show ? Visibility.Visible : Visibility.Collapsed;
                btnOcr.Content = show ? "Скрыть текст" : "Показать текст";
                btnCopyText.Visibility = show ? Visibility.Visible : Visibility.Collapsed;
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
            btnOcr.Content = "Распознать";

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
            btnOcr.Content = "Скрыть текст";
            btnCopyText.Visibility = Visibility.Visible;
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

        void OnLayerDown(object sender, MouseButtonEventArgs e)
        {
            if (words.Count == 0) return;
            Point p = e.GetPosition(ocrLayer);
            int idx = HitWord(p, 4);

            if (e.ClickCount >= 3) { SelectAll(); return; }
            if (e.ClickCount == 2 && idx >= 0)
            {
                int line = words[idx].Line;
                Select(Enumerable.Range(0, words.Count).Where(i => words[i].Line == line));
                return;
            }

            ocrLayer.CaptureMouse();
            if (idx >= 0)
            {
                dragWords = true;
                anchor = idx;
                Select(new[] { idx });
            }
            else
            {
                dragMarquee = true;
                marqueeStart = p;
                Select(new int[0]);
                marquee.Width = 0;
                marquee.Height = 0;
                Canvas.SetLeft(marquee, p.X);
                Canvas.SetTop(marquee, p.Y);
                marquee.Visibility = Visibility.Visible;
            }
        }

        void OnLayerMove(object sender, MouseEventArgs e)
        {
            Point p = e.GetPosition(ocrLayer);
            if (dragWords)
            {
                int j = NearestWord(p);
                if (j >= 0) Select(Enumerable.Range(Math.Min(anchor, j), Math.Abs(anchor - j) + 1));
            }
            else if (dragMarquee)
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
            }
        }

        void OnLayerUp(object sender, MouseButtonEventArgs e)
        {
            dragWords = false;
            dragMarquee = false;
            marquee.Visibility = Visibility.Collapsed;
            ocrLayer.ReleaseMouseCapture();
            if (selected.Count > 0) SetStatus("Выделено слов: " + selected.Count + " · Ctrl+C — копировать текст");
        }

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
    }
}
