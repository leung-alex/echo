// Explicitly authorized integration input; separate from Echo's no-clipboard test bridge.
// Every input targets the PID/title of a harness-owned process and verifies foreground first.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Web.Script.Serialization;
public static class EchoFocusDriver {
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Gui { public uint Size,Flags; public IntPtr Active,Focus,Capture,MenuOwner,MoveSize,Caret; public Rect CaretRect; }
    [StructLayout(LayoutKind.Sequential)] struct Monitor { public uint Size; public Rect Bounds,Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] struct Keyboard { public ushort Key,Scan; public uint Flags,Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int X,Y; public uint Data,Flags,Time; public UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)] struct InputData { [FieldOffset(0)] public Keyboard Keyboard; [FieldOffset(0)] public Mouse Mouse; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public InputData Data; }
    [DllImport("user32.dll",SetLastError=true)] static extern bool RegisterHotKey(IntPtr hwnd,int id,uint modifiers,uint key);
    [DllImport("user32.dll",SetLastError=true)] static extern bool UnregisterHotKey(IntPtr hwnd,int id);
    [DllImport("user32.dll",SetLastError=true)] static extern uint SendInput(uint count,Input[] inputs,int size);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint pid);
    [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread,ref Gui info);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr hwnd,ref Point point);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
    [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr hwnd,uint flags);
    [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr monitor,ref Monitor info);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool LogicalToPhysicalPointForPerMonitorDPI(IntPtr hwnd,ref Point point);
    [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr monitor,uint type,out uint x,out uint y);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int w,int h,uint flags);
    static IntPtr Owned(string root,int pid,string title,bool visible=true) {
        var entries=Json.Deserialize<Dictionary<string,object>[]>(File.ReadAllText(Path.Combine(root,"owned-processes.json")));
        using(var process=Process.GetProcessById(pid)) {
            bool match=false;
            foreach(var entry in entries) {
                if(Convert.ToInt32(entry["pid"])==pid && (string)entry["title"]==title &&
                   entry.ContainsKey("executable") && !String.IsNullOrWhiteSpace(entry["executable"] as string) &&
                   String.Equals(Path.GetFullPath((string)entry["executable"]),Path.GetFullPath(process.MainModule.FileName),StringComparison.OrdinalIgnoreCase) &&
                   Math.Abs((process.StartTime.ToUniversalTime()-DateTime.Parse((string)entry["started_utc"]).ToUniversalTime()).TotalSeconds)<1) match=true;
            }
            if(!match)throw new InvalidOperationException("Input process does not belong to this isolated run.");
        }
        var hwnd=EchoUi.Window(pid,title,visible);
        if(hwnd==IntPtr.Zero)throw new InvalidOperationException("Owned visible window missing.");
        return hwnd;
    }
    static uint[] Chord(string text) {
        uint mods=0,key=0;
        foreach(var token in text.ToUpperInvariant().Split('+')) {
            switch(token) {case "CTRL":mods|=2;break;case "ALT":mods|=1;break;case "SHIFT":mods|=4;break;default:
                if(token.Length==1 && Char.IsLetterOrDigit(token[0])) key=token[0];
                else throw new ArgumentException("Integration driver only supports Ctrl/Alt/Shift plus A-Z/0-9.");break;}
        }
        if(mods==0||key==0)throw new ArgumentException("Invalid test shortcut.");
        return new uint[]{mods,key};
    }
    static Input Key(uint key,bool up) {return new Input{Type=1,Data=new InputData{Keyboard=new Keyboard{Key=(ushort)key,Flags=up?2U:0U}}};}
    static void Send(List<Input> list) {
        uint sent=SendInput((uint)list.Count,list.ToArray(),Marshal.SizeOf(typeof(Input)));
        if(sent==list.Count)return;
        var pressed=new List<ushort>();
        for(int i=0;i<Math.Min((int)sent,list.Count);i++) {
            var key=list[i].Data.Keyboard;
            if((key.Flags&2)!=0)pressed.Remove(key.Key);
            else if(!pressed.Contains(key.Key))pressed.Add(key.Key);
        }
        var release=new List<Input>();for(int i=pressed.Count-1;i>=0;i--)release.Add(Key(pressed[i],true));
        if(release.Count>0)SendInput((uint)release.Count,release.ToArray(),Marshal.SizeOf(typeof(Input)));
        throw new InvalidOperationException("Windows rejected synthetic input; only keys confirmed injected by this call were released.");
    }
    static void Hotkey(IntPtr expected,string chord,int repeats,string holdRoot) {
        foreach(var vk in new[]{0x10,0x11,0x12,0x5b,0x5c}) if((GetAsyncKeyState(vk)&0x8000)!=0)throw new InvalidOperationException("Physical modifier busy; test input cancelled.");
        if(GetForegroundWindow()!=expected)throw new InvalidOperationException("Foreground changed; test input cancelled.");
        var c=Chord(chord);if((GetAsyncKeyState((int)c[1])&0x8000)!=0)throw new InvalidOperationException("Physical shortcut key busy; test input cancelled.");var modifiers=new List<uint>();
        if((c[0]&2)!=0)modifiers.Add(0x11);if((c[0]&1)!=0)modifiers.Add(0x12);if((c[0]&4)!=0)modifiers.Add(0x10);
        var inputs=new List<Input>();foreach(var key in modifiers)inputs.Add(Key(key,false));
        for(int i=0;i<Math.Max(1,Math.Min(30,repeats));i++)inputs.Add(Key(c[1],false));
        inputs.Add(Key(c[1],true));
        if(holdRoot==null){for(int i=modifiers.Count-1;i>=0;i--)inputs.Add(Key(modifiers[i],true));Send(inputs);return;}
        try {
            Send(inputs);File.WriteAllText(Path.Combine(holdRoot,"held.ready"),"test-owned modifiers");
            var watch=Stopwatch.StartNew();while(!File.Exists(Path.Combine(holdRoot,"release-held"))&&watch.ElapsedMilliseconds<12000)Thread.Sleep(10);
        }finally{var release=new List<Input>();for(int i=modifiers.Count-1;i>=0;i--)release.Add(Key(modifiers[i],true));Send(release);}
    }
    static object Geometry(IntPtr hwnd) {
        Rect rect;if(!GetWindowRect(hwnd,out rect))throw new InvalidOperationException("Window rectangle unavailable.");
        var monitor=new Monitor{Size=(uint)Marshal.SizeOf(typeof(Monitor))};if(!GetMonitorInfo(MonitorFromWindow(hwnd,2),ref monitor))throw new InvalidOperationException("Monitor unavailable.");
        uint pid;var thread=GetWindowThreadProcessId(hwnd,out pid);var gui=new Gui{Size=(uint)Marshal.SizeOf(typeof(Gui))};int[] caret=null;
        if(GetGUIThreadInfo(thread,ref gui)&&gui.Caret!=IntPtr.Zero){var a=new Point{X=gui.CaretRect.Left,Y=gui.CaretRect.Top};var b=new Point{X=gui.CaretRect.Right,Y=gui.CaretRect.Bottom};var old=SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(gui.Caret));try{if(ClientToScreen(gui.Caret,ref a)&&ClientToScreen(gui.Caret,ref b)&&LogicalToPhysicalPointForPerMonitorDPI(gui.Caret,ref a)&&LogicalToPhysicalPointForPerMonitorDPI(gui.Caret,ref b))caret=new[]{a.X,a.Y,b.X,b.Y};}finally{SetThreadDpiAwarenessContext(old);}}
        uint dx,dy;GetDpiForMonitor(MonitorFromWindow(hwnd,2),0,out dx,out dy);
        return new{monitor_dpi=dx,window=new[]{rect.Left,rect.Top,rect.Right,rect.Bottom},work=new[]{monitor.Work.Left,monitor.Work.Top,monitor.Work.Right,monitor.Work.Bottom},dpi=GetDpiForWindow(hwnd),caret=caret,foreground=GetForegroundWindow()==hwnd};
    }
    static object Probe(string chord) {
        var c=Chord(chord);bool registered=RegisterHotKey(IntPtr.Zero,201,c[0]|0x4000,c[1]);int error=registered?0:Marshal.GetLastWin32Error();
        if(registered&&!UnregisterHotKey(IntPtr.Zero,201))throw new InvalidOperationException("Probe registration did not release.");
        return new{available=registered,error=error};
    }
    static object Cycles(string root,int pid,string title,int count) {
        Owned(root,pid,title);if(count<1||count>100)throw new ArgumentException("Bounded cycle count required.");
        var samples=new List<double>();using(var process=Process.GetProcessById(pid)) {
            string executable=process.MainModule.FileName;
            for(int i=0;i<count+2;i++) {
                EchoUi.Close(pid,title);var wait=Stopwatch.StartNew();
                while(EchoUi.Window(pid,title,true)!=IntPtr.Zero&&wait.ElapsedMilliseconds<3000)Thread.Sleep(2);
                if(EchoUi.Window(pid,title,true)!=IntPtr.Zero)throw new TimeoutException("Hide did not complete.");
                var timer=Stopwatch.StartNew();using(var secondary=Process.Start(new ProcessStartInfo(executable,"--history"){UseShellExecute=false,CreateNoWindow=true})) {
                    if(!secondary.WaitForExit(5000)||secondary.ExitCode!=0)throw new InvalidOperationException("Reactivation handoff failed.");
                }
                while(!EchoUi.Ready(pid,title)&&timer.ElapsedMilliseconds<5000)Thread.Sleep(2);
                if(!EchoUi.Ready(pid,title)||EchoUi.WindowCount(pid)!=1)throw new InvalidOperationException("Single-window semantic readiness failed.");
                if(i>=2)samples.Add(timer.Elapsed.TotalMilliseconds);
            }
            EchoUi.Close(pid,title);Thread.Sleep(1500);process.Refresh();var before=process.TotalProcessorTime;Thread.Sleep(2000);process.Refresh();
            return new{samples_ms=samples,private_bytes=process.PrivateMemorySize64,working_set_bytes=process.WorkingSet64,threads=process.Threads.Count,handles=process.HandleCount,
                hidden_cpu_ms_over_2s=(process.TotalProcessorTime-before).TotalMilliseconds,note="Release manager reopen via secondary process to UIA semantic readiness; not physical hotkey/first-present latency. Two warmups excluded."};
        }
    }
    static object Block(string root,string chord) {
        var c=Chord(chord);if(!RegisterHotKey(IntPtr.Zero,202,c[0]|0x4000,c[1]))throw new InvalidOperationException("Cannot reserve conflict fixture shortcut.");
        try{File.WriteAllText(Path.Combine(root,"blocker.ready"),chord);var clock=Stopwatch.StartNew();while(!File.Exists(Path.Combine(root,"stop-blocker"))&&clock.Elapsed.TotalMinutes<20)Thread.Sleep(25);}
        finally{if(!UnregisterHotKey(IntPtr.Zero,202))throw new InvalidOperationException("Conflict fixture did not release.");}
        return new{released=true};
    }
    static object HotkeyReady(string root,IntPtr target,string chord,int echoPid,string echoTitle) {
        Owned(root,echoPid,echoTitle,false);
        var clock=Stopwatch.StartNew();Hotkey(target,chord,1,null);
        while(clock.ElapsedMilliseconds<5000) {
            if(EchoUi.Ready(echoPid,echoTitle))return new{elapsed_ms=clock.Elapsed.TotalMilliseconds,note="Synthetic OS hotkey to UIA semantic readiness; includes UIA observer cost, not display first presentation."};
            Thread.Sleep(2);
        }
        throw new TimeoutException("Hotkey did not produce semantic readiness.");
    }
    public static int Main(string[] args) {
        Console.OutputEncoding=System.Text.Encoding.UTF8;
        try {
            if(Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE")!="1")throw new InvalidOperationException("Explicit native acceptance authorization required.");
            if(args.Length<2)throw new ArgumentException("operation and evidence root required.");
            string root=Path.GetFullPath(args[1]);
            var marker=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(Path.Combine(root,"data","synthetic-fixture.json")));
            if(!Convert.ToBoolean(marker["synthetic"])||Convert.ToBoolean(marker["capture_enabled"]))throw new InvalidOperationException("Only capture-disabled synthetic fixtures are allowed.");
            SetProcessDpiAwarenessContext(new IntPtr(-4));SetThreadDpiAwarenessContext(new IntPtr(-4));object result=null;
            if(args[0]=="probe")result=Probe(args[2]);
            else if(args[0]=="block")result=Block(root,args[2]);
            else {
                int pid=Int32.Parse(args[2]);string title=args[3];var hwnd=Owned(root,pid,title);
                switch(args[0]) {
                    case "geometry":result=Geometry(hwnd);break;
                    case "read-edit":result=EchoUi.ReadText(pid,title,args[4]);break;
                    case "focus-edit-permitted":
                        string permit=Path.Combine(root,"foreground-permit-"+Process.GetCurrentProcess().Id);
                        var permissionWait=Stopwatch.StartNew();while(!File.Exists(permit)&&permissionWait.ElapsedMilliseconds<4000)Thread.Sleep(5);
                        if(!File.Exists(permit))throw new InvalidOperationException("Owned foreground grant was not issued.");
                        File.Delete(permit);EchoUi.Focus(pid,title);goto case "focus-edit";
                    case "focus-edit":EchoUi.Find(pid,title,args[4]).SetFocus();var wait=Stopwatch.StartNew();while(GetForegroundWindow()!=hwnd && wait.ElapsedMilliseconds<1500)Thread.Sleep(10);if(GetForegroundWindow()!=hwnd)throw new InvalidOperationException("Owned input did not obtain foreground");result=Geometry(hwnd);break;
                    case "card":var b=EchoUi.Find(pid,title,args[4]).Current.BoundingRectangle;result=new[]{b.Left,b.Top,b.Right,b.Bottom};break;
                    case "hotkey":Hotkey(hwnd,args[4],args.Length>5?Int32.Parse(args[5]):1,null);result=new{sent=true};break;
                    case "hotkey-ready":result=HotkeyReady(root,hwnd,args[4],Int32.Parse(args[5]),args[6]);break;
                    case "hold-hotkey":Hotkey(hwnd,args[4],1,root);result=new{released=true};break;
                    case "move":if(!SetWindowPos(hwnd,IntPtr.Zero,Int32.Parse(args[4]),Int32.Parse(args[5]),0,0,0x0001|0x0004|0x0010))throw new InvalidOperationException("Owned window move failed.");result=Geometry(hwnd);break;
                    case "cycles":result=Cycles(root,pid,title,Int32.Parse(args[4]));break;
                    default:throw new ArgumentException("Unknown bounded integration operation.");
                }
            }
            Console.WriteLine(Json.Serialize(new{status="PASS",value=result}));return 0;
        }catch(Exception error){Console.Error.WriteLine(Json.Serialize(new{status="FAIL",error=error.ToString()}));return 1;}
    }
}
