using System;
using System.Drawing;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Threading;

namespace ThodShot
{
    /// <summary>Фоновая часть: значок в трее, перехват PrintScreen, запуск снимка.</summary>
    internal sealed class Resident
    {
        public static Resident Instance { get; private set; }

        readonly Mutex mutex;
        Application app;
        System.Windows.Forms.NotifyIcon tray;
        KeyboardHook hook;
        CaptureOverlay overlay;
        PreviewWindow preview;
        SettingsWindow settingsWin;
        bool capturing;
        readonly RegisteredWaitHandle[] waits = new RegisteredWaitHandle[4];
        readonly EventWaitHandle[] events = new EventWaitHandle[4];

        public Resident(Mutex mutex)
        {
            this.mutex = mutex;
            Instance = this;
        }

        public int Run(string cmd)
        {
            AppSettings.Current = AppSettings.Load();

            app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
            app.DispatcherUnhandledException += (s, e) => { Log.Write(e.Exception.ToString()); e.Handled = true; };

            Icon icon = null;
            try { icon = Icon.ExtractAssociatedIcon(Environment.ProcessPath); } catch { }
            tray = new System.Windows.Forms.NotifyIcon
            {
                Icon = icon ?? SystemIcons.Application,
                Text = "ThodShot — скриншоты",
                Visible = true
            };
            var menu = new System.Windows.Forms.ContextMenuStrip();
            menu.Items.Add("Сделать снимок", null, (s, e) => Dispatch(StartCapture));
            menu.Items.Add("Настройки", null, (s, e) => Dispatch(ShowSettings));
            menu.Items.Add(new System.Windows.Forms.ToolStripSeparator());
            menu.Items.Add("Выйти", null, (s, e) => Dispatch(Exit));
            tray.ContextMenuStrip = menu;
            tray.MouseClick += (s, e) => { if (e.Button == System.Windows.Forms.MouseButtons.Left) Dispatch(StartCapture); };

            hook = new KeyboardHook(() => Dispatch(StartCapture));
            ApplyHook();

            // прогреваем захват, чтобы PrintScreen срабатывал без задержки
            Task.Run(() => WgcCapture.Warmup());

            RegisterEvent(0, Ipc.CaptureEvt, StartCapture);
            RegisterEvent(1, Ipc.SettingsEvt, ShowSettings);
            RegisterEvent(2, Ipc.ReloadEvt, Reload);
            RegisterEvent(3, Ipc.ExitEvt, Exit);

            app.Dispatcher.BeginInvoke(new Action(() =>
            {
                if (cmd == "--capture") StartCapture();
                else if (cmd != "--background") ShowSettings();
            }));

            app.Run();

            foreach (var w in waits) if (w != null) w.Unregister(null);
            hook.Dispose();
            tray.Visible = false;
            tray.Dispose();
            return 0;
        }

        void RegisterEvent(int i, string name, Action action)
        {
            events[i] = new EventWaitHandle(false, EventResetMode.AutoReset, name);
            waits[i] = ThreadPool.RegisterWaitForSingleObject(events[i], (state, timedOut) => Dispatch(action), null, -1, false);
        }

        void Dispatch(Action a)
        {
            if (app != null) app.Dispatcher.BeginInvoke(a);
        }

        public void ApplyHook()
        {
            if (AppSettings.Current.InterceptPrintScreen) hook.Start(); else hook.Stop();
        }

        void Reload()
        {
            AppSettings.Current = AppSettings.Load();
            ApplyHook();
        }

        public void OnSettingsChanged()
        {
            ApplyHook();
        }

        void Exit()
        {
            try { if (overlay != null) overlay.Close(); } catch { }
            app.Shutdown();
        }

        public void ShowSettings()
        {
            if (settingsWin == null)
            {
                settingsWin = new SettingsWindow();
                settingsWin.Closed += (s, e) => settingsWin = null;
                settingsWin.Show();
            }
            if (settingsWin.WindowState == WindowState.Minimized) settingsWin.WindowState = WindowState.Normal;
            settingsWin.Activate();
            Native.ForceForeground(new WindowInteropHelper(settingsWin).Handle);
        }

        public void StartCapture()
        {
            if (capturing) return;
            capturing = true;
            try
            {
                if (preview != null) { preview.Close(); preview = null; }
            }
            catch { }

            // Захват (особенно WGC) не должен подвешивать окно и хуки — делаем его в фоне.
            Task.Run(() =>
            {
                try
                {
                    Rectangle bounds;
                    Bitmap shot = ScreenGrab.CaptureVirtualScreen(out bounds);
                    Dispatch(() => BeginOverlay(shot, bounds));
                }
                catch (Exception ex)
                {
                    Log.Write("Ошибка съёмки: " + ex);
                    Dispatch(() =>
                    {
                        capturing = false;
                        ShowCaptureError(ex);
                    });
                }
            });
        }

        void BeginOverlay(Bitmap shot, Rectangle bounds)
        {
            try
            {
                overlay = new CaptureOverlay(shot, bounds, AppSettings.Current.DefaultMode);
                overlay.Completed += bmp =>
                {
                    capturing = false;
                    overlay = null;
                    ShowPreview(bmp);
                };
                overlay.Cancelled += () =>
                {
                    capturing = false;
                    overlay = null;
                };
                overlay.Present();
            }
            catch (Exception ex)
            {
                try { shot.Dispose(); } catch { }
                capturing = false;
                overlay = null;
                Log.Write("Ошибка оверлея: " + ex);
                ShowCaptureError(ex);
            }
        }

        static void ShowCaptureError(Exception ex)
        {
            try
            {
                System.Windows.Forms.MessageBox.Show("Не удалось сделать снимок: " + ex.Message,
                    "ThodShot", System.Windows.Forms.MessageBoxButtons.OK, System.Windows.Forms.MessageBoxIcon.Error);
            }
            catch { }
        }

        void ShowPreview(Bitmap bmp)
        {
            try
            {
                preview = new PreviewWindow(bmp);
                var created = preview;
                preview.Closed += (s, e) => { if (ReferenceEquals(preview, created)) preview = null; };
                preview.Show();
                // вывести окно поверх остальных, даже если система не отдаёт фокус
                preview.Topmost = true;
                preview.Topmost = false;
                preview.Activate();
                Native.ForceForeground(new WindowInteropHelper(preview).Handle);
            }
            catch (Exception ex) { Log.Write("Ошибка предпросмотра: " + ex); }
        }
    }
}
