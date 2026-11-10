using System;
using System.Diagnostics;
using System.Threading;
using Microsoft.Win32;

namespace ThodShot
{
    internal static class Ipc
    {
        public const string MutexName = @"Local\ThodShot.Instance.v1";
        public const string CaptureEvt = @"Local\ThodShot.Capture";
        public const string SettingsEvt = @"Local\ThodShot.Settings";
        public const string ReloadEvt = @"Local\ThodShot.Reload";
        public const string ExitEvt = @"Local\ThodShot.Exit";

        public static bool IsRunning()
        {
            Mutex m;
            if (Mutex.TryOpenExisting(MutexName, out m)) { m.Dispose(); return true; }
            return false;
        }

        public static void Signal(string name)
        {
            using (var e = new EventWaitHandle(false, EventResetMode.AutoReset, name)) e.Set();
        }

        public static bool StopRunning(int timeoutMs)
        {
            if (!IsRunning()) return true;
            Signal(ExitEvt);
            var sw = Stopwatch.StartNew();
            while (sw.ElapsedMilliseconds < timeoutMs)
            {
                if (!IsRunning()) return true;
                Thread.Sleep(100);
            }
            return false;
        }
    }

    internal static class Integration
    {
        const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
        const string RunName = "ThodShot";
        const string KbdKey = @"Control Panel\Keyboard";
        const string PrtScValue = "PrintScreenKeyForSnippingEnabled";

        /// <summary>Применяет настройки к системе (автозапуск, PrintScreen) и сохраняет файл настроек.</summary>
        public static void Apply(AppSettings s)
        {
            try
            {
                using (var k = Registry.CurrentUser.CreateSubKey(RunKey))
                {
                    if (s.Autostart)
                        k.SetValue(RunName, "\"" + Environment.ProcessPath + "\" --background", RegistryValueKind.String);
                    else
                        k.DeleteValue(RunName, false);
                }
            }
            catch (Exception ex) { Log.Write("Автозапуск: " + ex.Message); }

            try
            {
                using (var kb = Registry.CurrentUser.OpenSubKey(KbdKey, true))
                {
                    if (kb != null)
                    {
                        if (s.InterceptPrintScreen)
                        {
                            if (!s.PrtScRecorded)
                            {
                                object v = kb.GetValue(PrtScValue);
                                s.PrtScOriginal = v is int ? (int?)(int)v : null;
                                s.PrtScRecorded = true;
                            }
                            kb.SetValue(PrtScValue, 0, RegistryValueKind.DWord);
                        }
                        else RestorePrtSc(s, kb);
                    }
                }
            }
            catch (Exception ex) { Log.Write("PrintScreen: " + ex.Message); }
            s.Save();
        }

        static void RestorePrtSc(AppSettings s, RegistryKey kb)
        {
            if (!s.PrtScRecorded) return;
            if (s.PrtScOriginal.HasValue) kb.SetValue(PrtScValue, s.PrtScOriginal.Value, RegistryValueKind.DWord);
            else kb.DeleteValue(PrtScValue, false);
            s.PrtScRecorded = false;
            s.PrtScOriginal = null;
        }

        /// <summary>Удаляет всё, что программа добавила в систему. Настройки и данные остаются.</summary>
        public static int Cleanup()
        {
            try
            {
                Ipc.StopRunning(15000);
                var s = AppSettings.Load();
                using (var kb = Registry.CurrentUser.OpenSubKey(KbdKey, true))
                {
                    if (kb != null) RestorePrtSc(s, kb);
                }
                using (var k = Registry.CurrentUser.OpenSubKey(RunKey, true))
                {
                    if (k != null) k.DeleteValue(RunName, false);
                }
                s.Save();
                Paths.ForgetPointer();
                return 0;
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(ex.Message);
                Log.Write("Cleanup: " + ex);
                return 1;
            }
        }
    }
}
