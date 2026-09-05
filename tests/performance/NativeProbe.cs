using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class EchoProbeNative {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc fn, IntPtr p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, System.Text.StringBuilder b, int n);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    public class Window {public long Handle; public string Title; public bool Visible; public Rect Bounds;}
    public static Window[] Windows(int pid) {
        var list=new List<Window>();
        EnumWindows((h,p)=> { uint owner; GetWindowThreadProcessId(h,out owner);
            if(owner==(uint)pid) {var text=new System.Text.StringBuilder(512); GetWindowText(h,text,text.Capacity);
                Rect r; GetWindowRect(h,out r); list.Add(new Window{Handle=h.ToInt64(),Title=text.ToString(),Visible=IsWindowVisible(h),Bounds=r});}
            return true;}, IntPtr.Zero);
        return list.ToArray();
    }
}
