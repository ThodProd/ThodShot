using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Threading.Tasks;
using Windows.Globalization;
using Windows.Graphics.Imaging;
using Windows.Media.Ocr;
using Windows.Storage.Streams;

namespace ThodShot
{
    internal sealed class OcrWordInfo
    {
        public string Text;
        public double X, Y, W, H;
        public int Line;
    }

    internal sealed class OcrOutcome
    {
        public List<OcrWordInfo> Words = new List<OcrWordInfo>();
        public bool NoLanguage;
        public string Error;
    }

    /// <summary>Локальное распознавание текста встроенным движком Windows (без интернета).</summary>
    internal static class OcrService
    {
        static List<OcrEngine> GetEngines(string setting)
        {
            var list = new List<OcrEngine>();
            Action<string> add = tag =>
            {
                try
                {
                    var lang = new Language(tag);
                    if (OcrEngine.IsLanguageSupported(lang))
                    {
                        var e = OcrEngine.TryCreateFromLanguage(lang);
                        if (e != null) list.Add(e);
                    }
                }
                catch { }
            };
            if (setting == "ru") add("ru");
            else if (setting == "en") add("en-US");
            else { add("ru"); add("en-US"); }

            if (list.Count == 0)
            {
                try
                {
                    var e = OcrEngine.TryCreateFromUserProfileLanguages();
                    if (e != null) list.Add(e);
                }
                catch { }
            }
            return list;
        }

        public static async Task<OcrOutcome> RecognizeAsync(byte[] png, string languageSetting)
        {
            var outcome = new OcrOutcome();
            var engines = GetEngines(languageSetting ?? "auto");
            if (engines.Count == 0)
            {
                outcome.NoLanguage = true;
                return outcome;
            }

            using (var ras = new InMemoryRandomAccessStream())
            {
                await ras.WriteAsync(png.AsBuffer());
                ras.Seek(0);
                var decoder = await BitmapDecoder.CreateAsync(ras);
                uint w = decoder.PixelWidth, h = decoder.PixelHeight;
                double scale = 1.0;
                double maxSide = Math.Max(w, h);
                if (maxSide < 1200) scale = 2.0;
                if (maxSide * scale > OcrEngine.MaxImageDimension) scale = OcrEngine.MaxImageDimension / maxSide;

                var transform = new BitmapTransform
                {
                    ScaledWidth = (uint)Math.Max(1, Math.Round(w * scale)),
                    ScaledHeight = (uint)Math.Max(1, Math.Round(h * scale)),
                    InterpolationMode = BitmapInterpolationMode.Fant
                };

                using (var bitmap = await decoder.GetSoftwareBitmapAsync(
                    BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, transform,
                    ExifOrientationMode.IgnoreExifOrientation, ColorManagementMode.DoNotColorManage))
                {
                    OcrResult best = null;
                    int bestScore = -1;
                    foreach (var engine in engines)
                    {
                        var res = await engine.RecognizeAsync(bitmap);
                        int score = (res.Text ?? "").Count(char.IsLetterOrDigit);
                        if (score > bestScore) { best = res; bestScore = score; }
                    }

                    if (best != null)
                    {
                        int li = 0;
                        foreach (var line in best.Lines)
                        {
                            foreach (var word in line.Words)
                            {
                                var r = word.BoundingRect;
                                outcome.Words.Add(new OcrWordInfo
                                {
                                    Text = word.Text,
                                    X = r.X / scale,
                                    Y = r.Y / scale,
                                    W = r.Width / scale,
                                    H = r.Height / scale,
                                    Line = li
                                });
                            }
                            li++;
                        }
                    }
                }
            }
            return outcome;
        }
    }
}
