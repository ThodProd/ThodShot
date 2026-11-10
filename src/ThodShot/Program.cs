using System;
using System.Diagnostics;
using System.IO;
using System.Threading;

namespace ThodShot
{
    internal static class Program
    {
        [STAThread]
        private static int Main(string[] args)
        {
            string cmd = args.Length > 0 ? args[0] : "";
            try
            {
                Paths.Init();

                switch (cmd)
                {
                    case "--register-integrations":
                        {
                            var s = AppSettings.Load();
                            Integration.Apply(s);
                            return 0;
                        }
                    case "--cleanup":
                        return Integration.Cleanup();
                    case "--stop-background":
                        return Ipc.StopRunning(15000) ? 0 : 1;
                    case "--restart":
                        {
                            // дождаться выхода текущего экземпляра и открыть настройки заново
                            for (int i = 0; i < 150 && Ipc.IsRunning(); i++) Thread.Sleep(100);
                            StartProcess("--settings");
                            return 0;
                        }
                    case "--enable-printscreen":
                    case "--disable-printscreen":
                        {
                            var s = AppSettings.Load();
                            s.InterceptPrintScreen = cmd == "--enable-printscreen";
                            Integration.Apply(s);
                            if (Ipc.IsRunning()) Ipc.Signal(Ipc.ReloadEvt);
                            else if (s.InterceptPrintScreen) StartBackground();
                            return 0;
                        }
                }

                bool created;
                var mutex = new Mutex(true, Ipc.MutexName, out created);
                if (!created)
                {
                    // программа уже работает в трее — передаём ей команду
                    if (cmd == "--capture") Ipc.Signal(Ipc.CaptureEvt);
                    else if (cmd != "--background") Ipc.Signal(Ipc.SettingsEvt);
                    return 0;
                }

                var resident = new Resident(mutex);
                return resident.Run(cmd);
            }
            catch (Exception ex)
            {
                Log.Write("Критическая ошибка: " + ex);
                Console.Error.WriteLine(ex.Message);
                return 1;
            }
        }

        private static void StartBackground()
        {
            StartProcess("--background");
        }

        private static void StartProcess(string arguments)
        {
            try
            {
                var psi = new ProcessStartInfo(Environment.ProcessPath, arguments)
                {
                    UseShellExecute = false,
                    WorkingDirectory = Path.GetDirectoryName(Environment.ProcessPath)
                };
                Process.Start(psi);
            }
            catch (Exception ex) { Log.Write("Не удалось запустить процесс: " + ex.Message); }
        }
    }
}
