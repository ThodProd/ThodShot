using System;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace ThodShot
{
    internal static class Paths
    {
        public static string DataDir { get; private set; }
        public static string SettingsFile { get { return Path.Combine(DataDir, "settings.json"); } }
        public static string LogsDir { get { return Path.Combine(DataDir, "logs"); } }

        static string Pointer
        {
            get
            {
                return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "ThodShot", "datadir.txt");
            }
        }

        public static void Init()
        {
            string local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            string env = Environment.GetEnvironmentVariable("THODHUB_DATA_DIR");
            string dir = null;
            try
            {
                if (!string.IsNullOrWhiteSpace(env))
                {
                    dir = env;
                    // запоминаем каталог для запусков вне хаба (автозапуск, трей)
                    Directory.CreateDirectory(Path.GetDirectoryName(Pointer));
                    if (!File.Exists(Pointer) || File.ReadAllText(Pointer).Trim() != env)
                        File.WriteAllText(Pointer, env, new UTF8Encoding(false));
                }
                else if (File.Exists(Pointer))
                {
                    dir = File.ReadAllText(Pointer).Trim();
                }
            }
            catch { }
            if (string.IsNullOrWhiteSpace(dir)) dir = Path.Combine(local, "ThodHUB", "Data", "thodshot");
            DataDir = dir;
            try { Directory.CreateDirectory(DataDir); } catch { }
        }

        public static void ForgetPointer()
        {
            try { if (File.Exists(Pointer)) File.Delete(Pointer); } catch { }
        }
    }

    internal sealed class AppSettings
    {
        public string SaveFolder { get; set; } = "";
        public bool Autostart { get; set; } = true;
        public bool InterceptPrintScreen { get; set; } = true;
        public string DefaultMode { get; set; } = "region";   // region | full
        public string Theme { get; set; } = "dark";           // dark | light | system
        public bool OpenFolderAfterSave { get; set; } = true;
        public string Format { get; set; } = "png";           // png | jpg
        public string OcrLanguage { get; set; } = "auto";     // auto | ru | en

        // версия применённых миграций настроек
        public int SetupVersion { get; set; }

        // исходное значение системной настройки PrintScreen -> Snipping Tool
        public bool PrtScRecorded { get; set; }
        public int? PrtScOriginal { get; set; }

        [JsonIgnore]
        public string EffectiveFolder
        {
            get
            {
                if (!string.IsNullOrWhiteSpace(SaveFolder)) return SaveFolder;
                return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyPictures), "Скриншоты");
            }
        }

        public static AppSettings Current { get; set; } = new AppSettings();

        public static AppSettings Load()
        {
            try
            {
                if (File.Exists(Paths.SettingsFile))
                {
                    var s = JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(Paths.SettingsFile, Encoding.UTF8));
                    if (s != null)
                    {
                        s.Migrate();
                        return s;
                    }
                }
            }
            catch (Exception ex) { Log.Write("Ошибка чтения настроек: " + ex.Message); }
            return new AppSettings();
        }

        /// <summary>Разовые миграции настроек между версиями.</summary>
        void Migrate()
        {
            if (SetupVersion < 1)
            {
                // в 1.1 областью по умолчанию стала область, а не весь экран
                if (DefaultMode != "region" && DefaultMode != "full") DefaultMode = "region";
                else if (DefaultMode == "full") DefaultMode = "region";
                SetupVersion = 1;
                Save();
            }
        }

        public void Save()
        {
            try
            {
                Directory.CreateDirectory(Paths.DataDir);
                string json = JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true });
                string tmp = Paths.SettingsFile + ".tmp";
                File.WriteAllText(tmp, json, new UTF8Encoding(false));
                if (File.Exists(Paths.SettingsFile)) File.Replace(tmp, Paths.SettingsFile, null);
                else File.Move(tmp, Paths.SettingsFile);
            }
            catch (Exception ex) { Log.Write("Ошибка записи настроек: " + ex.Message); }
        }
    }

    internal static class Log
    {
        static readonly object gate = new object();
        public static void Write(string text)
        {
            try
            {
                lock (gate)
                {
                    Directory.CreateDirectory(Paths.LogsDir);
                    string f = Path.Combine(Paths.LogsDir, "thodshot.log");
                    if (File.Exists(f) && new FileInfo(f).Length > 512 * 1024) File.Delete(f);
                    File.AppendAllText(f, DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + " " + text + Environment.NewLine, Encoding.UTF8);
                }
            }
            catch { }
        }
    }
}
