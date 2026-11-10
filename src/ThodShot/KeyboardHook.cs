using System;
using System.Runtime.InteropServices;

namespace ThodShot
{
    /// <summary>Глобальный перехват клавиши PrintScreen (без модификаторов).</summary>
    internal sealed class KeyboardHook : IDisposable
    {
        const int WH_KEYBOARD_LL = 13;
        const int WM_KEYDOWN = 0x0100;
        const int VK_SNAPSHOT = 0x2C;

        readonly Native.HookProc proc;
        readonly Action onPress;
        IntPtr hook = IntPtr.Zero;
        bool isDown;

        public KeyboardHook(Action onPress)
        {
            this.onPress = onPress;
            proc = Callback; // держим ссылку, чтобы делегат не собрал GC
        }

        public bool Active { get { return hook != IntPtr.Zero; } }

        public void Start()
        {
            if (hook != IntPtr.Zero) return;
            hook = Native.SetWindowsHookEx(WH_KEYBOARD_LL, proc, Native.GetModuleHandle(null), 0);
            if (hook == IntPtr.Zero) Log.Write("Не удалось установить перехват клавиатуры: " + Marshal.GetLastWin32Error());
        }

        public void Stop()
        {
            if (hook == IntPtr.Zero) return;
            Native.UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
            isDown = false;
        }

        static bool Pressed(int vk) { return (Native.GetAsyncKeyState(vk) & 0x8000) != 0; }

        IntPtr Callback(int nCode, IntPtr wParam, IntPtr lParam)
        {
            if (nCode >= 0 && Marshal.ReadInt32(lParam) == VK_SNAPSHOT)
            {
                // Alt/Win/Ctrl/Shift+PrintScreen оставляем системе
                bool mods = Pressed(0x12) || Pressed(0x5B) || Pressed(0x5C) || Pressed(0x11) || Pressed(0x10);
                int msg = wParam.ToInt32();
                if (!mods && msg == WM_KEYDOWN)
                {
                    if (!isDown) { isDown = true; try { onPress(); } catch (Exception ex) { Log.Write(ex.ToString()); } }
                    return (IntPtr)1;
                }
                if (isDown && (msg == 0x0101 /* WM_KEYUP */ || msg == 0x0105 /* WM_SYSKEYUP */))
                {
                    isDown = false;
                    return (IntPtr)1;
                }
            }
            return Native.CallNextHookEx(hook, nCode, wParam, lParam);
        }

        public void Dispose() { Stop(); }
    }
}
