using System;
using System.Runtime.InteropServices;

namespace ThodShot
{
    internal static class Native
    {
        public delegate IntPtr HookProc(int nCode, IntPtr wParam, IntPtr lParam);

        [DllImport("user32.dll", SetLastError = true)]
        public static extern IntPtr SetWindowsHookEx(int idHook, HookProc proc, IntPtr hMod, uint threadId);
        [DllImport("user32.dll")]
        public static extern bool UnhookWindowsHookEx(IntPtr hhk);
        [DllImport("user32.dll")]
        public static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
        public static extern IntPtr GetModuleHandle(string name);
        [DllImport("user32.dll")]
        public static extern short GetAsyncKeyState(int vKey);
        [DllImport("user32.dll")]
        public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")]
        public static extern bool BringWindowToTop(IntPtr h);
        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")]
        public static extern bool AttachThreadInput(uint a, uint b, bool attach);
        [DllImport("kernel32.dll")]
        public static extern uint GetCurrentThreadId();
        [DllImport("user32.dll")]
        public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("user32.dll")]
        public static extern IntPtr GetDC(IntPtr hWnd);
        [DllImport("user32.dll")]
        public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);
        [DllImport("gdi32.dll", SetLastError = true)]
        public static extern bool BitBlt(IntPtr hdcDest, int xDest, int yDest, int width, int height, IntPtr hdcSrc, int xSrc, int ySrc, uint rop);

        public static void ForceForeground(IntPtr hwnd)
        {
            try
            {
                IntPtr fg = GetForegroundWindow();
                uint pid;
                uint cur = GetCurrentThreadId();
                uint fgThread = GetWindowThreadProcessId(fg, out pid);
                bool attached = fgThread != 0 && fgThread != cur && AttachThreadInput(cur, fgThread, true);
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
                if (attached) AttachThreadInput(cur, fgThread, false);
            }
            catch { }
        }
    }
}
