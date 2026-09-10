using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class EchoProbeNative {
    [DllImport("user32.dll", SetLastError=true)] static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern IntPtr GetClipboardOwner();
    [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll", SetLastError=true)] static extern IntPtr GetClipboardData(uint format);
    [DllImport("kernel32.dll", SetLastError=true)] static extern UIntPtr GlobalSize(IntPtr data);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr GlobalLock(IntPtr data);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr data);
    public class DibSnapshot {public uint Format;public ulong Bytes;public string Sha256;public string Path;}
    // Test publisher only: copy the actual Windows representation, after its
    // format conversion, rather than treating a pre-clipboard BMP as the source.
    public static DibSnapshot SaveOwnedDib(int publisher,string path) {
        if(Environment.GetEnvironmentVariable("ECHO_CLIPBOARD_BACKUP_READY")!="1") throw new InvalidOperationException("Clipboard preservation required");
        bool opened=false;
        for(int attempt=0;attempt<20 && !opened;attempt++) {opened=OpenClipboard(IntPtr.Zero);if(!opened)System.Threading.Thread.Sleep(10);}
        if(!opened) throw new InvalidOperationException("Synthetic clipboard busy");
        uint format=0;ulong size=0;
        try {
            uint owner;GetWindowThreadProcessId(GetClipboardOwner(),out owner);
            if(owner!=(uint)publisher || publisher<=0) throw new InvalidOperationException("Synthetic publisher no longer owns clipboard");
            format=IsClipboardFormatAvailable(17)?17u:8u;
            IntPtr handle=GetClipboardData(format);size=GlobalSize(handle).ToUInt64();
            if(handle==IntPtr.Zero || size<40 || size>256UL*1024*1024) throw new InvalidOperationException("Synthetic DIB outside test bound");
            IntPtr data=GlobalLock(handle);
            if(data==IntPtr.Zero) throw new InvalidOperationException("Synthetic DIB lock failed");
            try {
                byte[] buffer=new byte[1024*1024];
                using(var stream=new System.IO.FileStream(path,System.IO.FileMode.CreateNew,System.IO.FileAccess.Write)) {
                    for(int offset=0;offset<(int)size;) {
                        int count=Math.Min(buffer.Length,(int)size-offset);
                        Marshal.Copy(IntPtr.Add(data,offset),buffer,0,count);stream.Write(buffer,0,count);offset+=count;
                    }
                }
            } finally {GlobalUnlock(handle);}
        } finally {CloseClipboard();}
        using(var sha=System.Security.Cryptography.SHA256.Create())using(var stream=System.IO.File.OpenRead(path)) {
            return new DibSnapshot{Format=format,Bytes=size,Path=path,Sha256=BitConverter.ToString(sha.ComputeHash(stream)).Replace("-","").ToLowerInvariant()};
        }
    }
    [DllImport("user32.dll", SetLastError=true)] static extern bool RegisterHotKey(IntPtr window,int id,uint modifiers,uint key);
    [DllImport("user32.dll", SetLastError=true)] static extern bool UnregisterHotKey(IntPtr window,int id);
    public static bool AltVAvailable() {
        bool registered=RegisterHotKey(IntPtr.Zero,219,0x4001,0x56);
        if(registered && !UnregisterHotKey(IntPtr.Zero,219)) throw new InvalidOperationException("Hotkey probe failed to release registration");
        if(!registered && Marshal.GetLastWin32Error()!=1409) throw new InvalidOperationException("Hotkey probe failed without an existing registration");
        return registered;
    }
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
