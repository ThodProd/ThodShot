using System.Windows;
using System.Windows.Controls;
using System.Windows.Markup;
using System.Windows.Media;
using Microsoft.Win32;

namespace ThodShot
{
    internal static class Theme
    {
        public static readonly bool Dark;
        public static readonly string BgHex, SurfaceHex, TextHex, MutedHex, LineHex, AccentHex, AccentTextHex, HoverHex, AccentHoverHex;
        public static readonly SolidColorBrush Bg, Surface, Text, Muted, Line, Accent, AccentText;
        public static readonly SolidColorBrush Danger = Make("#B92F3A");

        static Theme()
        {
            string mode = "dark";
            try
            {
                if (AppSettings.Current != null && !string.IsNullOrWhiteSpace(AppSettings.Current.Theme))
                    mode = AppSettings.Current.Theme;
            }
            catch { }

            bool dark = mode == "dark";
            if (mode != "dark" && mode != "light")
            {
                try
                {
                    using (var k = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"))
                    {
                        object v = k != null ? k.GetValue("AppsUseLightTheme") : null;
                        if (v is int) dark = (int)v == 0;
                    }
                }
                catch { }
            }
            Dark = dark;
            if (dark)
            {
                BgHex = "#1C1D20"; SurfaceHex = "#25262A"; TextHex = "#ECEDF0"; MutedHex = "#989CA5";
                LineHex = "#303136"; AccentHex = "#81AAFF"; AccentTextHex = "#10223F"; HoverHex = "#2E2F34"; AccentHoverHex = "#9BBAFF";
            }
            else
            {
                BgHex = "#FFFFFF"; SurfaceHex = "#F7F8FA"; TextHex = "#24262B"; MutedHex = "#767B85";
                LineHex = "#E8E9ED"; AccentHex = "#3676E8"; AccentTextHex = "#FFFFFF"; HoverHex = "#ECEEF2"; AccentHoverHex = "#2C63C4";
            }
            Bg = Make(BgHex); Surface = Make(SurfaceHex); Text = Make(TextHex); Muted = Make(MutedHex);
            Line = Make(LineHex); Accent = Make(AccentHex); AccentText = Make(AccentTextHex);
        }

        public static SolidColorBrush Make(string hex)
        {
            var b = (SolidColorBrush)new BrushConverter().ConvertFromString(hex);
            b.Freeze();
            return b;
        }
    }

    internal static class Styles
    {
        static readonly Style[] cache = new Style[3];

        /// <summary>kind: 0 — обычная, 1 — главная (акцент), 2 — плоская (без рамки).</summary>
        public static Style Button(int kind)
        {
            if (cache[kind] != null) return cache[kind];
            string bg = kind == 1 ? Theme.AccentHex : kind == 2 ? "Transparent" : Theme.SurfaceHex;
            string fg = kind == 1 ? Theme.AccentTextHex : Theme.TextHex;
            string bd = kind == 0 ? Theme.LineHex : "Transparent";
            string hv = kind == 1 ? Theme.AccentHoverHex : Theme.HoverHex;
            string xaml =
                "<Style xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" " +
                "xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\" TargetType=\"Button\">" +
                "<Setter Property=\"Background\" Value=\"@BG@\"/>" +
                "<Setter Property=\"Foreground\" Value=\"@FG@\"/>" +
                "<Setter Property=\"BorderBrush\" Value=\"@BD@\"/>" +
                "<Setter Property=\"BorderThickness\" Value=\"1\"/>" +
                "<Setter Property=\"Padding\" Value=\"14,7\"/>" +
                "<Setter Property=\"Cursor\" Value=\"Hand\"/>" +
                "<Setter Property=\"FocusVisualStyle\" Value=\"{x:Null}\"/>" +
                "<Setter Property=\"Template\"><Setter.Value>" +
                "<ControlTemplate TargetType=\"Button\">" +
                "<Border x:Name=\"b\" Background=\"{TemplateBinding Background}\" BorderBrush=\"{TemplateBinding BorderBrush}\" " +
                "BorderThickness=\"{TemplateBinding BorderThickness}\" CornerRadius=\"8\" Padding=\"{TemplateBinding Padding}\">" +
                "<ContentPresenter HorizontalAlignment=\"Center\" VerticalAlignment=\"Center\"/></Border>" +
                "<ControlTemplate.Triggers>" +
                "<Trigger Property=\"IsMouseOver\" Value=\"True\"><Setter TargetName=\"b\" Property=\"Background\" Value=\"@HV@\"/></Trigger>" +
                "<Trigger Property=\"IsEnabled\" Value=\"False\"><Setter TargetName=\"b\" Property=\"Opacity\" Value=\"0.5\"/></Trigger>" +
                "</ControlTemplate.Triggers></ControlTemplate></Setter.Value></Setter></Style>";
            xaml = xaml.Replace("@BG@", bg).Replace("@FG@", fg).Replace("@BD@", bd).Replace("@HV@", hv);
            cache[kind] = (Style)XamlReader.Parse(xaml);
            return cache[kind];
        }
    }
}
