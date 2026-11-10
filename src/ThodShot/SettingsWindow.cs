using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls;

namespace ThodShot
{
    internal sealed class SettingsWindow : ThemedWindow
    {
        readonly TextBlock folderText = new TextBlock();

        public SettingsWindow() : base("Настройки · ThodShot")
        {
            Width = 540;
            Height = 640;
            MinWidth = 460;
            MinHeight = 420;
            WindowStartupLocation = WindowStartupLocation.CenterScreen;

            var panel = new StackPanel { Margin = new Thickness(24, 8, 24, 24) };
            Body.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = panel });

            var s = AppSettings.Current;

            // Папка
            panel.Children.Add(Section("Сохранение"));
            panel.Children.Add(Muted("Папка для снимков"));
            folderText.Text = s.EffectiveFolder;
            folderText.TextTrimming = TextTrimming.CharacterEllipsis;
            folderText.Margin = new Thickness(0, 4, 0, 8);
            panel.Children.Add(folderText);
            var row = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 12) };
            var pick = Btn("Выбрать папку", 0, PickFolder);
            var open = Btn("Открыть", 0, () =>
            {
                try
                {
                    Directory.CreateDirectory(AppSettings.Current.EffectiveFolder);
                    Process.Start(new ProcessStartInfo("explorer.exe", "\"" + AppSettings.Current.EffectiveFolder + "\"") { UseShellExecute = true });
                }
                catch (Exception ex) { Log.Write(ex.Message); }
            });
            open.Margin = new Thickness(8, 0, 0, 0);
            row.Children.Add(pick);
            row.Children.Add(open);
            panel.Children.Add(row);

            panel.Children.Add(ChoiceRow("Формат файла", new[] { "PNG", "JPG" }, new[] { "png", "jpg" },
                () => AppSettings.Current.Format, v => AppSettings.Current.Format = v));
            panel.Children.Add(ToggleRow("После сохранения открыть папку", null,
                () => AppSettings.Current.OpenFolderAfterSave, v => AppSettings.Current.OpenFolderAfterSave = v));

            // Съёмка
            panel.Children.Add(Section("Съёмка"));
            panel.Children.Add(ChoiceRow("Режим по умолчанию", new[] { "Область", "Весь экран" }, new[] { "region", "full" },
                () => AppSettings.Current.DefaultMode, v => AppSettings.Current.DefaultMode = v));
            panel.Children.Add(ToggleRow("Перехватывать PrintScreen",
                "Другие программы скриншотов по PrintScreen не откроются",
                () => AppSettings.Current.InterceptPrintScreen, v => AppSettings.Current.InterceptPrintScreen = v));
            panel.Children.Add(ToggleRow("Запускать вместе с Windows", "Программа работает в трее без окна",
                () => AppSettings.Current.Autostart, v => AppSettings.Current.Autostart = v));

            // Распознавание
            panel.Children.Add(Section("Распознавание текста"));
            panel.Children.Add(ChoiceRow("Язык", new[] { "Авто", "Русский", "English" }, new[] { "auto", "ru", "en" },
                () => AppSettings.Current.OcrLanguage, v => AppSettings.Current.OcrLanguage = v));
            panel.Children.Add(Muted("Работает без интернета на встроенном движке Windows. Нужен языковой пакет Windows."));

            var ver = Assembly.GetExecutingAssembly().GetName().Version;
            var foot = Muted("Версия " + ver.Major + "." + ver.Minor + "." + ver.Build + "  ·  Данные: " + Paths.DataDir);
            foot.Margin = new Thickness(0, 24, 0, 0);
            foot.FontSize = 12;
            panel.Children.Add(foot);
        }

        void Changed()
        {
            var s = AppSettings.Current;
            Integration.Apply(s); // применяет автозапуск и PrintScreen, сохраняет файл настроек
            if (Resident.Instance != null) Resident.Instance.OnSettingsChanged();
        }

        void PickFolder()
        {
            using (var d = new System.Windows.Forms.FolderBrowserDialog())
            {
                d.Description = "Папка для скриншотов";
                d.UseDescriptionForTitle = true;
                d.SelectedPath = AppSettings.Current.EffectiveFolder;
                if (d.ShowDialog() == System.Windows.Forms.DialogResult.OK)
                {
                    AppSettings.Current.SaveFolder = d.SelectedPath;
                    folderText.Text = d.SelectedPath;
                    Changed();
                }
            }
        }

        static TextBlock Section(string text)
        {
            return new TextBlock
            {
                Text = text,
                FontSize = 16,
                FontWeight = FontWeights.SemiBold,
                Margin = new Thickness(0, 20, 0, 8)
            };
        }

        static TextBlock Muted(string text)
        {
            return new TextBlock
            {
                Text = text,
                Foreground = Theme.Muted,
                FontSize = 13,
                TextWrapping = TextWrapping.Wrap
            };
        }

        static Button Btn(string text, int kind, Action click)
        {
            var b = new Button { Content = text, Style = Styles.Button(kind), Padding = new Thickness(12, 6, 12, 6) };
            b.Click += (s, e) => click();
            return b;
        }

        FrameworkElement Segmented(string[] labels, int selected, Action<int> onChange)
        {
            var sp = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
            var buttons = new List<Button>();
            for (int i = 0; i < labels.Length; i++)
            {
                int idx = i;
                var b = Btn(labels[i], i == selected ? 1 : 0, () => { });
                b.Margin = new Thickness(i == 0 ? 0 : 6, 0, 0, 0);
                b.Click += (s, e) =>
                {
                    for (int j = 0; j < buttons.Count; j++) buttons[j].Style = Styles.Button(j == idx ? 1 : 0);
                    onChange(idx);
                };
                buttons.Add(b);
                sp.Children.Add(b);
            }
            return sp;
        }

        FrameworkElement ChoiceRow(string title, string[] labels, string[] values, Func<string> get, Action<string> set)
        {
            var grid = new Grid { Margin = new Thickness(0, 6, 0, 6) };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var t = new TextBlock { Text = title, VerticalAlignment = VerticalAlignment.Center };
            int sel = Math.Max(0, Array.IndexOf(values, get()));
            var seg = Segmented(labels, sel, i => { set(values[i]); Changed(); });
            Grid.SetColumn(seg, 1);
            grid.Children.Add(t);
            grid.Children.Add(seg);
            return grid;
        }

        FrameworkElement ToggleRow(string title, string hint, Func<bool> get, Action<bool> set)
        {
            var grid = new Grid { Margin = new Thickness(0, 6, 0, 6) };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var left = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 12, 0) };
            left.Children.Add(new TextBlock { Text = title, TextWrapping = TextWrapping.Wrap });
            if (hint != null) left.Children.Add(Muted(hint));
            var seg = Segmented(new[] { "Вкл", "Выкл" }, get() ? 0 : 1, i => { set(i == 0); Changed(); });
            Grid.SetColumn(seg, 1);
            grid.Children.Add(left);
            grid.Children.Add(seg);
            return grid;
        }
    }
}
