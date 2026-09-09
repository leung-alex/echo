// Explicitly authorized integration input; separate from Echo's no-clipboard test bridge.
// Every input targets the PID/title of a harness-owned process and verifies foreground first.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
using System.Web.Script.Serialization;
public static class EchoInlineDriver {
    static long lastInputTimestamp;
    [DllImport("gdi32.dll",SetLastError=true)] static extern bool StretchBlt(IntPtr dest,int x,int y,int width,int height,IntPtr source,int sx,int sy,int sw,int sh,uint operation);
    [DllImport("winmm.dll")] static extern uint timeBeginPeriod(uint period);
    [DllImport("winmm.dll")] static extern uint timeEndPeriod(uint period);
    static object MeasureOpen(string root, IntPtr input, string name, int duration) {
        if(name.IndexOfAny(Path.GetInvalidFileNameChars())>=0)throw new ArgumentException("Invalid sample name");
        duration=Math.Max(1000,Math.Min(10000,duration));
        Guard(input);
        var monitor=new Monitor{Size=(uint)Marshal.SizeOf(typeof(Monitor))};
        if(!GetMonitorInfo(MonitorFromWindow(input,2),ref monitor))throw new InvalidOperationException("Monitor unavailable");
        var r=monitor.Bounds;const int factor=4;
        string regionFile=Path.Combine(root,"capture-region.json");
        if(File.Exists(regionFile)) {
            var region=Json.Deserialize<int[]>(File.ReadAllText(regionFile));
            if(region.Length!=4)throw new ArgumentException("Invalid capture region");
            r.Left=Math.Max(r.Left,region[0]);r.Top=Math.Max(r.Top,region[1]);
            r.Right=Math.Min(r.Right,region[2]);r.Bottom=Math.Min(r.Bottom,region[3]);
            if(r.Right-r.Left<64||r.Bottom-r.Top<64)throw new ArgumentException("Empty capture region");
        }
        int width=(r.Right-r.Left)/factor,height=(r.Bottom-r.Top)/factor;
        var frames=new List<byte[]>();var times=new List<double[]>();
        IntPtr desktop=GetDC(IntPtr.Zero);
        bool timerResolution=timeBeginPeriod(1)==0;
        try {
        using(var scaled=new Bitmap(width,height,PixelFormat.Format32bppArgb))
        using(var resize=Graphics.FromImage(scaled)) {
            Action sample=()=>{
                var dest=resize.GetHdc();
                try { if(!StretchBlt(dest,0,0,width,height,desktop,r.Left,r.Top,r.Right-r.Left,r.Bottom-r.Top,0x00CC0020))throw new InvalidOperationException("Desktop capture failed"); }
                finally {resize.ReleaseHdc(dest);}
                var bits=scaled.LockBits(new Rectangle(0,0,width,height),ImageLockMode.ReadOnly,PixelFormat.Format32bppArgb);
                try {var bytes=new byte[width*height*4];Marshal.Copy(bits.Scan0,bytes,0,bytes.Length);frames.Add(bytes);}
                finally {scaled.UnlockBits(bits);}
            };
            sample();times.Add(new[]{-1.0,-1.0});
            Hotkey(input,"Alt+V",1,null);
            long start=lastInputTimestamp;
            Func<double> elapsed=()=>1000.0*(Stopwatch.GetTimestamp()-start)/Stopwatch.Frequency;
            while(elapsed()<duration) {
                if(GetForegroundWindow()!=input)throw new InvalidOperationException("Foreground changed during timing");
                double before=elapsed();sample();times.Add(new[]{before,elapsed()});Thread.Sleep(1);
            }
        }
        } finally {if(timerResolution)timeEndPeriod(1);ReleaseDC(IntPtr.Zero,desktop);}
        string output=Path.Combine(root,name+".bgra.gz");
        using(var stream=new GZipStream(File.Create(output),CompressionMode.Compress))
            foreach(var frame in frames)stream.Write(frame,0,frame.Length);
        var result=new {name=name,width=width,height=height,origin=new[]{r.Left,r.Top},factor=factor,
            format="BGRA32",frames=times,frequency=Stopwatch.Frequency,input_qpc=lastInputTimestamp,
            source="desktop StretchBlt; timestamps bracket each capture; t0 immediately before SendInput(Alt+V)",file=output};
        File.WriteAllText(Path.Combine(root,name+".frames.json"),Json.Serialize(result));
        return new {file=output,frames=frames.Count,duration_ms=times[times.Count-1][1]};
    }

    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    static Dictionary<string,object> applicationTarget;
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
        string registered=Path.Combine(root,"application-input-targets.json");
        if(File.Exists(registered)) {
            var targets=Json.Deserialize<Dictionary<string,object>[]>(File.ReadAllText(registered));
            foreach(var target in targets) {
                if(Convert.ToInt32(target["pid"])!=pid||(string)target["title"]!=title)continue;
                using(var process=Process.GetProcessById(pid)) {
                    if(!String.Equals(Path.GetFullPath((string)target["executable"]),Path.GetFullPath(process.MainModule.FileName),StringComparison.OrdinalIgnoreCase)||
                       process.StartTime.ToUniversalTime().ToFileTimeUtc()!=Convert.ToInt64(target["started_filetime"]))throw new InvalidOperationException("Registered application process identity changed.");
                }
                long handle=Convert.ToInt64(target["hwnd"]);
                foreach(object forbidden in (System.Collections.IEnumerable)target["forbidden_windows"])if(Convert.ToInt64(forbidden)==handle)throw new InvalidOperationException("Executing task window is forbidden.");
                EchoUi.BindWindow(pid,title,handle);
                var selected=EchoUi.Window(pid,title,visible);
                if(selected==IntPtr.Zero)throw new InvalidOperationException("Registered application HWND is unavailable.");
                applicationTarget=target;
                return selected;
            }
        }
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
    static string VerifyApplicationComposer(int pid,string title,string operation) {
        if(applicationTarget==null)return null;
        if(operation!="hotkey"&&operation!="paced-hotkey"&&operation!="key"&&operation!="text"&&operation!="held-enter"&&operation!="geometry"&&operation!="application-state")throw new InvalidOperationException("This operation is not allowed on a shared application process.");
        Guard(EchoUi.Window(pid,title,true));
        bool draft=false;System.Windows.Automation.AutomationElement editor=null;
        foreach(var element in EchoUi.Elements(pid,title)) {
            if(element.Current.ControlType==System.Windows.Automation.ControlType.Text&&element.Current.Name.Trim()==((string)applicationTarget["draft_marker"]).Trim())draft=true;
            if(element.Current.ControlType==System.Windows.Automation.ControlType.Edit&&element.Current.Name==(string)applicationTarget["composer_name"]&&element.Current.HasKeyboardFocus) {
                if(editor!=null)throw new InvalidOperationException("More than one focused composer.");editor=element;
            }
        }
        if(!draft||editor==null)throw new InvalidOperationException("The explicit window is not the authorized blank-draft composer with input focus (draft="+draft+", focused_editor="+(editor!=null)+"); no input sent.");
        var value=((System.Windows.Automation.ValuePattern)editor.GetCurrentPattern(System.Windows.Automation.ValuePattern.Pattern)).Current.Value;
        bool permitted=false;foreach(object expected in (System.Collections.IEnumerable)applicationTarget["allowed_values"])if((string)expected==value)permitted=true;
        if(!permitted)throw new InvalidOperationException("Composer text differs from this run's allowed synthetic values; no input sent.");
        return value;
    }
    static uint[] Chord(string text) {
        uint mods=0,key=0;
        foreach(var token in text.ToUpperInvariant().Split('+')) {
            switch(token) {case "CTRL":mods|=2;break;case "ALT":mods|=1;break;case "SHIFT":mods|=4;break;case "TAB":key=9;break;default:
                if(token.Length==1 && Char.IsLetterOrDigit(token[0])) key=token[0];
                else throw new ArgumentException("Integration driver only supports Ctrl/Alt/Shift plus A-Z/0-9 or Tab.");break;}
        }
        if(mods==0||key==0)throw new ArgumentException("Invalid test shortcut.");
        return new uint[]{mods,key};
    }
    static Input Key(uint key,bool up) {return new Input{Type=1,Data=new InputData{Keyboard=new Keyboard{Key=(ushort)key,Flags=up?2U:0U}}};}
    static void Send(List<Input> list) {
        lastInputTimestamp = Stopwatch.GetTimestamp();
        uint sent=SendInput((uint)list.Count,list.ToArray(),Marshal.SizeOf(typeof(Input)));
        if(sent==list.Count)return;
        var pressed=new List<Keyboard>();bool mouseDown=false;
        for(int i=0;i<Math.Min((int)sent,list.Count);i++) {
            if(list[i].Type==0){if((list[i].Data.Mouse.Flags&2)!=0)mouseDown=true;if((list[i].Data.Mouse.Flags&4)!=0)mouseDown=false;continue;}
            var key=list[i].Data.Keyboard;
            if((key.Flags&2)!=0)pressed.RemoveAll(k=>k.Key==key.Key&&k.Scan==key.Scan&&(k.Flags&4)==(key.Flags&4));
            else if(!pressed.Exists(k=>k.Key==key.Key&&k.Scan==key.Scan&&(k.Flags&4)==(key.Flags&4)))pressed.Add(key);
        }
        var release=new List<Input>();
        for(int i=pressed.Count-1;i>=0;i--){var key=pressed[i];key.Flags|=2;release.Add(new Input{Type=1,Data=new InputData{Keyboard=key}});}
        if(mouseDown)release.Add(new Input{Type=0,Data=new InputData{Mouse=new Mouse{Flags=4}}});
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
    [DllImport("user32.dll")] static extern bool PostMessageW(IntPtr hwnd,uint message,IntPtr w,IntPtr l);
    [DllImport("user32.dll")] static extern int GetKeyboardLayoutList(int count,IntPtr[] layouts);
    static object EnglishOwned(IntPtr hwnd) {
        int count=GetKeyboardLayoutList(0,null);var layouts=new IntPtr[count];GetKeyboardLayoutList(count,layouts);
        foreach(var layout in layouts) {
            if((layout.ToInt64()&0xffff)==0x0409){
                if(!PostMessageW(hwnd,0x0050,IntPtr.Zero,layout))throw new InvalidOperationException("Could not request the installed English layout for the owned test window");
                Thread.Sleep(80);return new {requested=true,scope="owned fixture thread",layout=layout.ToInt64()};
            }
        }
        return new {requested=false,scope="owned fixture thread",layout=0L};
    }
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd,uint flags);
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr hwnd,int index);
    static object ClickOwned(int pid,string title,string label,IntPtr hwnd,bool activate) {
        IntPtr foregroundBefore=GetForegroundWindow();
        foreach(var vk in new[]{0x01,0x02,0x10,0x11,0x12,0x5b,0x5c})if((GetAsyncKeyState(vk)&0x8000)!=0)throw new InvalidOperationException("A physical button/modifier is held; pointer test cancelled");
        if(activate&&!SetWindowPos(hwnd,new IntPtr(-1),0,0,0,0,0x0001|0x0002|0x0010))throw new InvalidOperationException("Could not raise the owned fixture without activation");
        var bounds=EchoUi.Find(pid,title,label).Current.BoundingRectangle;
        var point=new Point{X=(int)(bounds.Left+bounds.Width/2),Y=(int)(bounds.Top+bounds.Height/2)};
        var ready=Stopwatch.StartNew();
        while(GetAncestor(WindowFromPoint(point),2)!=hwnd && ready.ElapsedMilliseconds<2000)Thread.Sleep(20);
        if(bounds.IsEmpty||bounds.Width<1||bounds.Height<1||GetAncestor(WindowFromPoint(point),2)!=hwnd){
            Rect window;GetWindowRect(hwnd,out window);uint foundPid;GetWindowThreadProcessId(WindowFromPoint(point),out foundPid);
            throw new InvalidOperationException("Owned point unavailable: control="+bounds+" point="+point.X+","+point.Y+" window="+window.Left+","+window.Top+","+window.Right+","+window.Bottom+" targetPid="+pid+" hitPid="+foundPid+" foregroundOwned="+(GetForegroundWindow()==hwnd));
        }
        Point old;GetCursorPos(out old);
        IntPtr foregroundAtClick=GetForegroundWindow();
        long extendedStyle=GetWindowLongPtr(hwnd,-20).ToInt64();
        if(!activate&&(foregroundAtClick==IntPtr.Zero||foregroundAtClick!=foregroundBefore))
            throw new InvalidOperationException("Foreground changed during read-only click targeting: before="+foregroundBefore+" after="+foregroundAtClick);
        if(!activate&&(extendedStyle&0x08000000)==0)
            throw new InvalidOperationException("Inline window lost WS_EX_NOACTIVATE before pointer input");
        try {
            if(!SetCursorPos(point.X,point.Y)||GetAncestor(WindowFromPoint(point),2)!=hwnd)throw new InvalidOperationException("Owned pointer target changed before click");
            // Bind each queued button event to the verified physical point.
            // SendInput queues delivery: an immediate SetCursorPos restore must
            // not redirect a pending button event to the user's prior window.
            int left=GetSystemMetrics(76),top=GetSystemMetrics(77),width=GetSystemMetrics(78),height=GetSystemMetrics(79);
            if(width<1||height<1)throw new InvalidOperationException("Virtual desktop geometry unavailable");
            int x=(int)(((long)(point.X-left)*65536+32768)/width),y=(int)(((long)(point.Y-top)*65536+32768)/height);
            var list=new List<Input>{new Input{Type=0,Data=new InputData{Mouse=new Mouse{X=x,Y=y,Flags=0xc003}}},new Input{Type=0,Data=new InputData{Mouse=new Mouse{X=x,Y=y,Flags=0xc005}}}};
            Send(list);
            Thread.Sleep(100);
            var watch=Stopwatch.StartNew();while(activate&&GetForegroundWindow()!=hwnd&&watch.ElapsedMilliseconds<1500)Thread.Sleep(10);
            if(activate&&GetForegroundWindow()!=hwnd)throw new InvalidOperationException("Physical click did not activate the owned fixture");
            return new {foreground_before=foregroundBefore.ToInt64(),foreground_at_click=foregroundAtClick.ToInt64(),foreground_after=GetForegroundWindow().ToInt64(),extended_style=extendedStyle,x=point.X,y=point.Y,window=hwnd.ToInt64()};
        } finally {Point current;GetCursorPos(out current);if(current.X==point.X&&current.Y==point.Y)SetCursorPos(old.X,old.Y);if(activate)SetWindowPos(hwnd,new IntPtr(-2),0,0,0,0,0x0001|0x0002|0x0010);}
    }
    static void Guard(IntPtr hwnd) {
        var foreground=GetForegroundWindow();
        if(foreground!=hwnd) {
            uint expectedPid,actualPid;
            GetWindowThreadProcessId(hwnd,out expectedPid);GetWindowThreadProcessId(foreground,out actualPid);
            throw new InvalidOperationException("Owned target lost foreground; input was not sent (expected_hwnd="+hwnd.ToInt64()+", expected_pid="+expectedPid+", actual_hwnd="+foreground.ToInt64()+", actual_pid="+actualPid+")");
        }
        foreach(var vk in new[]{0x10,0x11,0x12,0x5b,0x5c})if((GetAsyncKeyState(vk)&0x8000)!=0)throw new InvalidOperationException("A physical modifier is held; input cancelled");
    }
    static object ClickPoint(IntPtr hwnd, int x, int y) {
        var point=new Point{X=x,Y=y};Rect bounds;
        if(!GetWindowRect(hwnd,out bounds)||x<bounds.Left||x>=bounds.Right||y<bounds.Top||y>=bounds.Bottom
            ||GetAncestor(WindowFromPoint(point),2)!=hwnd)
            throw new InvalidOperationException("Point is not in the owned window's native input region");
        foreach(var vk in new[]{0x01,0x02,0x10,0x11,0x12,0x5b,0x5c})
            if((GetAsyncKeyState(vk)&0x8000)!=0)throw new InvalidOperationException("A physical button/modifier is held");
        Point old;GetCursorPos(out old);
        try {
            SetCursorPos(x,y);
            if(GetAncestor(WindowFromPoint(point),2)!=hwnd)throw new InvalidOperationException("Owned point moved before click");
            // Bind queued clicks to the verified point before restoring the cursor.
            int left=GetSystemMetrics(76),top=GetSystemMetrics(77),width=GetSystemMetrics(78),height=GetSystemMetrics(79);
            if(width<1||height<1)throw new InvalidOperationException("Virtual desktop geometry unavailable");
            int absoluteX=(int)(((long)(x-left)*65536+32768)/width),absoluteY=(int)(((long)(y-top)*65536+32768)/height);
            var click=new[]{new Input{Type=0,Data=new InputData{Mouse=new Mouse{X=absoluteX,Y=absoluteY,Flags=0xc003}}},new Input{Type=0,Data=new InputData{Mouse=new Mouse{X=absoluteX,Y=absoluteY,Flags=0xc005}}}};
            if(SendInput((uint)click.Length,click,Marshal.SizeOf(typeof(Input)))!=click.Length)throw new InvalidOperationException("Click failed");
            Thread.Sleep(100);
            return new{x=x,y=y,owner=hwnd.ToInt64()};
        } finally {Point current;GetCursorPos(out current);if(current.X==x&&current.Y==y)SetCursorPos(old.X,old.Y);}
    }
    static object ScreenOwned(string root, IntPtr hwnd, string name) {
        if(Path.GetFileName(name)!=name)throw new ArgumentException("Capture name must be local");
        Rect r;if(!GetWindowRect(hwnd,out r)||!IsWindowVisible(hwnd))throw new InvalidOperationException("Owned window is hidden");
        int width=r.Right-r.Left,height=r.Bottom-r.Top;
        if(width<1||height<1||width>4096||height>4096)throw new InvalidOperationException("Capture dimensions exceed budget");
        string path=Path.Combine(root,name);
        using(var bitmap=new Bitmap(width,height))using(var graphics=Graphics.FromImage(bitmap)) {
            graphics.CopyFromScreen(r.Left,r.Top,0,0,bitmap.Size,CopyPixelOperation.SourceCopy);
            bitmap.Save(path,ImageFormat.Png);
        }
        return new{file=path,bounds=new[]{r.Left,r.Top,r.Right,r.Bottom},source="desktop pixels within the owned window"};
    }
    static void Text(IntPtr hwnd,string text,bool enter) {
        Guard(hwnd);if(text.Length>4096)throw new ArgumentException("Bounded synthetic text required");
        var list=new List<Input>();
        foreach(char ch in text){list.Add(new Input{Type=1,Data=new InputData{Keyboard=new Keyboard{Scan=ch,Flags=4}}});list.Add(new Input{Type=1,Data=new InputData{Keyboard=new Keyboard{Scan=ch,Flags=6}}});}
        if(enter){list.Add(Key(13,false));list.Add(Key(13,true));}
        Send(list);
    }
    static void OneKey(IntPtr hwnd,uint key,int repeat,bool shift) {
        Guard(hwnd);if((GetAsyncKeyState((int)key)&0x8000)!=0)throw new InvalidOperationException("Requested physical key is held");
        var list=new List<Input>();if(shift)list.Add(Key(0x10,false));
        for(int i=0;i<Math.Max(1,Math.Min(30,repeat));i++)list.Add(Key(key,false));
        list.Add(Key(key,true));if(shift)list.Add(Key(0x10,true));Send(list);
    }
    static void HoldEnter(IntPtr hwnd) {
        Guard(hwnd);if((GetAsyncKeyState(13)&0x8000)!=0)throw new InvalidOperationException("Physical Enter is held");
        try {
            Send(new List<Input>{Key(13,false)});
            for(int i=0;i<12;i++){Thread.Sleep(50);if(GetForegroundWindow()!=hwnd)throw new InvalidOperationException("Input focus moved during held Enter");Send(new List<Input>{Key(13,false)});}
        } finally {Send(new List<Input>{Key(13,true)});}
    }
    static object ReadControl(int pid,string title,string name) {
        var element=EchoUi.Find(pid,title,name);object value;
        if(element.TryGetCurrentPattern(System.Windows.Automation.ValuePattern.Pattern,out value))return ((System.Windows.Automation.ValuePattern)value).Current.Value;
        if(element.TryGetCurrentPattern(System.Windows.Automation.TextPattern.Pattern,out value))return ((System.Windows.Automation.TextPattern)value).DocumentRange.GetText(-1);
        return element.Current.Name;
    }
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hwnd,IntPtr hdc);
    [DllImport("gdi32.dll")] static extern uint GetPixel(IntPtr hdc,int x,int y);
    static object SampleHeaders(string root,IntPtr echo,int pid,string title,int inputPid,string inputTitle) {
        IntPtr input=Owned(root,inputPid,inputTitle);
        if(GetForegroundWindow()!=input)throw new InvalidOperationException("Owned input is not foreground");
        var card=EchoUi.Find(pid,title,"History space").Current.BoundingRectangle;
        Rect initial;GetWindowRect(echo,out initial);
        int ox=(int)(card.Left-initial.Left),oy=(int)(card.Top-initial.Top);
        var frames=new List<object>();var watch=Stopwatch.StartNew();
        var bitmap=new System.Drawing.Bitmap((int)card.Width,1);
        var graphics=System.Drawing.Graphics.FromImage(bitmap);
        try {
            File.WriteAllText(Path.Combine(root,"pixels.ready"),"owned opaque header pixels only; no text or desktop capture");
            while(watch.ElapsedMilliseconds<12000&&!File.Exists(Path.Combine(root,"pixels.stop"))) {
                if(GetForegroundWindow()!=input||EchoUi.Window(pid,title,true)!=echo)throw new InvalidOperationException("Sampled session lost visibility or foreground");
                Rect rect;GetWindowRect(echo,out rect);var colors=new List<uint>();
                // Nine points in the solid header, above all text and icons.
                graphics.CopyFromScreen(rect.Left+ox,rect.Top+oy+8,0,0,new System.Drawing.Size(bitmap.Width,1),System.Drawing.CopyPixelOperation.SourceCopy);
                for(int x=1;x<=9;x++){var c=bitmap.GetPixel(bitmap.Width*x/10,0);colors.Add((uint)(c.R|(c.G<<8)|(c.B<<16)));}
                frames.Add(new{ms=watch.Elapsed.TotalMilliseconds,colors=colors});Thread.Sleep(8);
            }
        }finally{graphics.Dispose();bitmap.Dispose();}
        return new{source="physical screen samples of the owned Echo opaque header; no text pixels",frames=frames};
    }
    static object SampleActions(string root,IntPtr echo,int pid,string title,int inputPid,string inputTitle) {
        IntPtr input=Owned(root,inputPid,inputTitle);
        if(GetForegroundWindow()!=input)throw new InvalidOperationException("Owned input is not foreground");
        var row=EchoUi.Find(pid,title,"echo-perf-text-0013 — Reusable content, available when you need it.").Current.BoundingRectangle;
        Point oldPointer;GetCursorPos(out oldPointer);
        var hover=new Point{X=(int)(row.Left+row.Width/2),Y=(int)(row.Top+row.Height/2)};
        foreach(var key in new[]{0x01,0x02,0x10,0x11,0x12,0x5b,0x5c})if((GetAsyncKeyState(key)&0x8000)!=0)throw new InvalidOperationException("Physical pointer/modifier busy; hover sampling cancelled");
        if(GetAncestor(WindowFromPoint(hover),2)!=echo||!SetCursorPos(hover.X,hover.Y))throw new InvalidOperationException("Owned action hover point unavailable");
        Thread.Sleep(150);
        System.Windows.Rect icon;
        try {
            icon=EchoUi.Find(pid,title,"Copy item").Current.BoundingRectangle;
        } catch {
            Point current;GetCursorPos(out current);
            if(current.X==hover.X&&current.Y==hover.Y)SetCursorPos(oldPointer.X,oldPointer.Y);
            throw;
        }
        hover=new Point{X=(int)(icon.Left+icon.Width/2),Y=(int)(icon.Top+icon.Height/2)};
        if(GetForegroundWindow()!=input||GetAncestor(WindowFromPoint(hover),2)!=echo||!SetCursorPos(hover.X,hover.Y))
            throw new InvalidOperationException("Owned copy-action hover is unavailable");
        Thread.Sleep(150);
        Rect initial;GetWindowRect(echo,out initial);
        int ox=(int)(icon.Left-initial.Left),oy=(int)(icon.Top-initial.Top);
        var frames=new List<object>();var watch=Stopwatch.StartNew();
        try {
        using(var bitmap=new System.Drawing.Bitmap((int)icon.Width,(int)icon.Height))
        using(var graphics=System.Drawing.Graphics.FromImage(bitmap)) {
            File.WriteAllText(Path.Combine(root,"actions.ready"),"owned copy button samples only");
            while(watch.ElapsedMilliseconds<12000&&!File.Exists(Path.Combine(root,"actions.stop"))) {
                if(GetForegroundWindow()!=input||EchoUi.Window(pid,title,true)!=echo)throw new InvalidOperationException("Action sample lost target focus");
                Point pointer;GetCursorPos(out pointer);
                if(pointer.X!=hover.X||pointer.Y!=hover.Y)throw new InvalidOperationException("Stationary-hover acceptance interrupted by pointer movement");
                if(GetAncestor(WindowFromPoint(pointer),2)!=echo)throw new InvalidOperationException("Owned copy action was occluded during sampling");
                Rect rect;GetWindowRect(echo,out rect);
                graphics.CopyFromScreen(rect.Left+ox,rect.Top+oy,0,0,bitmap.Size,System.Drawing.CopyPixelOperation.SourceCopy);
                var colors=new List<int>();
                for(int y=7;y<bitmap.Height-7;y+=2)for(int x=7;x<bitmap.Width-7;x+=2)colors.Add(bitmap.GetPixel(x,y).ToArgb());
                frames.Add(new{ms=watch.ElapsedMilliseconds,colors=colors});Thread.Sleep(10);
            }
        }
        } finally {Point current;GetCursorPos(out current);if(current.X==hover.X&&current.Y==hover.Y)SetCursorPos(oldPointer.X,oldPointer.Y);}
        return new{source="physical pixels within owned Echo Copy action",stationary_hover=true,frames=frames};
    }
    static object RecordWindow(string root,IntPtr echo,int pid,string title,int inputPid,string inputTitle,string folder) {
        if(folder.IndexOfAny(Path.GetInvalidFileNameChars())>=0||folder=="."||folder=="..")throw new ArgumentException("Simple recording name required");
        var output=Path.Combine(root,folder);Directory.CreateDirectory(output);
        IntPtr input=Owned(root,inputPid,inputTitle);
        var frames=new List<object>();var watch=Stopwatch.StartNew();int count=0;
        File.WriteAllText(Path.Combine(output,"ready"),"physical screen pixels inside the owned synthetic Echo window");
        Bitmap bitmap=null;Graphics graphics=null;
        // UIA fault/recovery observations can outlast twelve seconds. The runner
        // stops recording at scenario completion; this is only a leak watchdog.
        try { while(watch.ElapsedMilliseconds<60000&&!File.Exists(Path.Combine(output,"stop"))) {
            uint owner;GetWindowThreadProcessId(echo,out owner);
            if(GetForegroundWindow()!=input||owner!=pid||!IsWindowVisible(echo))throw new InvalidOperationException("Recording lost the owned input or Echo window");
            Rect rect;if(!GetWindowRect(echo,out rect))throw new InvalidOperationException("Echo bounds unavailable");
            int width=rect.Right-rect.Left,height=rect.Bottom-rect.Top;
            if(width<=0||height<=0||width>4096||height>4096)throw new InvalidOperationException("Recording bounds exceed the owned window budget");
            string file=count.ToString("D4")+".jpg";long stamp=watch.ElapsedMilliseconds;
            if(bitmap==null||bitmap.Width!=width||bitmap.Height!=height) {
                if(graphics!=null)graphics.Dispose();if(bitmap!=null)bitmap.Dispose();
                bitmap=new Bitmap(width,height);graphics=Graphics.FromImage(bitmap);
            }
            graphics.CopyFromScreen(rect.Left,rect.Top,0,0,bitmap.Size,System.Drawing.CopyPixelOperation.SourceCopy);
            bitmap.Save(Path.Combine(output,file),System.Drawing.Imaging.ImageFormat.Jpeg);
            frames.Add(new{ms=stamp,file=file,bounds=new[]{rect.Left,rect.Top,rect.Right,rect.Bottom},foreground=input.ToInt64()});
            count++;int remaining=(int)(count*1000L/40-watch.ElapsedMilliseconds);if(remaining>0)Thread.Sleep(remaining);
        } } finally {if(graphics!=null)graphics.Dispose();if(bitmap!=null)bitmap.Dispose();}
        var result=new{source="physical screen; complete owned Echo window; JPEG frames",elapsed_ms=watch.ElapsedMilliseconds,frames=frames};
        File.WriteAllText(Path.Combine(output,"frames.json"),Json.Serialize(result),new System.Text.UTF8Encoding(false));
        return new{count=count,elapsed_ms=watch.ElapsedMilliseconds};
    }
    static void PacedHotkey(IntPtr expected,string chord,int delay) {
        if(delay<1||delay>500)throw new ArgumentException("Bounded key transition delay required.");
        Guard(expected);var c=Chord(chord);var pressed=new List<uint>();var modifiers=new List<uint>();
        if((c[0]&2)!=0)modifiers.Add(0x11);if((c[0]&1)!=0)modifiers.Add(0x12);if((c[0]&4)!=0)modifiers.Add(0x10);
        if((GetAsyncKeyState((int)c[1])&0x8000)!=0)throw new InvalidOperationException("Physical shortcut key is busy.");
        try {
            foreach(var key in modifiers){if(GetForegroundWindow()!=expected)throw new InvalidOperationException("Foreground changed during paced chord.");Send(new List<Input>{Key(key,false)});pressed.Add(key);Thread.Sleep(delay);}
            if(GetForegroundWindow()!=expected)throw new InvalidOperationException("Foreground changed during paced chord.");
            Send(new List<Input>{Key(c[1],false)});pressed.Add(c[1]);Thread.Sleep(delay);
            Send(new List<Input>{Key(c[1],true)});pressed.Remove(c[1]);Thread.Sleep(delay);
        }finally{var release=new List<Input>();for(int i=pressed.Count-1;i>=0;i--)release.Add(Key(pressed[i],true));if(release.Count>0)Send(release);}
    }
    delegate bool EnumWindowProc(IntPtr window,IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowProc callback,IntPtr parameter);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,System.Text.StringBuilder name,int maximum);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint command);
    [DllImport("imm32.dll",SetLastError=true)] static extern IntPtr ImmGetContext(IntPtr window);
    [DllImport("imm32.dll",SetLastError=true)] static extern int ImmGetCompositionStringW(IntPtr context,uint index,IntPtr buffer,uint bytes);
    [DllImport("imm32.dll")] static extern bool ImmReleaseContext(IntPtr window,IntPtr context);
    static object ForeignImeProbe(IntPtr expected) {
        Guard(expected);uint pid;uint tid=GetWindowThreadProcessId(expected,out pid);
        var gui=new Gui{Size=(uint)Marshal.SizeOf(typeof(Gui))};
        if(!GetGUIThreadInfo(tid,ref gui))throw new InvalidOperationException("Focused control unavailable");
        var context=ImmGetContext(gui.Focus);int bytes=-1;int error=Marshal.GetLastWin32Error();
        if(context!=IntPtr.Zero)try{bytes=ImmGetCompositionStringW(context,8,IntPtr.Zero,0);error=Marshal.GetLastWin32Error();}
        finally{ImmReleaseContext(gui.Focus,context);}
        return new{window=gui.Focus.ToInt64(),context=context.ToInt64(),composition_bytes=bytes,error=error,
            note="Read-only cross-process capability observation; not a production authorization source"};
    }
    static object ImeWindowMetadata(IntPtr expected) {
        Guard(expected);var items=new List<object>();
        EnumWindows(delegate(IntPtr window,IntPtr unused){
            if(!IsWindowVisible(window))return true;
            var name=new System.Text.StringBuilder(256);GetClassNameW(window,name,name.Capacity);
            uint pid;GetWindowThreadProcessId(window,out pid);Rect rect;GetWindowRect(window,out rect);
            items.Add(new{hwnd=window.ToInt64(),pid=pid,cls=name.ToString(),owner=GetWindow(window,4).ToInt64(),bounds=new[]{rect.Left,rect.Top,rect.Right,rect.Bottom}});return true;
        },IntPtr.Zero);return items;
    }
    [STAThread] public static int Main(string[] args) {
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
                string applicationValue=VerifyApplicationComposer(pid,title,args[0]);
                switch(args[0]) {
                    case "application-state":if(applicationTarget==null)throw new InvalidOperationException("An explicit application draft registration is required");result=new{draft=true,focused=true,text=applicationValue};break;
                    case "foreign-ime-probe":result=ForeignImeProbe(hwnd);break;
                    case "activate-owned":ClickOwned(pid,title,args[4],hwnd,true);result=Geometry(hwnd);break;
                    case "click-owned":result=ClickOwned(pid,title,args[4],hwnd,false);break;
                    case "click-point":result=ClickPoint(hwnd,Int32.Parse(args[4]),Int32.Parse(args[5]));break;
                    case "screen-owned":result=ScreenOwned(root,hwnd,args[4]);break;
                    case "english-owned":result=EnglishOwned(hwnd);break;
                    case "close-owned":if(!PostMessageW(hwnd,0x0010,IntPtr.Zero,IntPtr.Zero))throw new InvalidOperationException("Owned close message failed");result=new{requested=true};break;
                    case "text":Text(hwnd,args[4],false);result=new{sent=true};break;
                    case "text-enter":Text(hwnd,args[4],true);result=new{sent=true};break;
                    case "key":OneKey(hwnd,UInt32.Parse(args[4]),args.Length>5?Int32.Parse(args[5]):1,args.Length>6&&args[6]=="shift");result=new{sent=true};break;
                    case "held-enter":HoldEnter(hwnd);result=new{sent=true};break;
                    case "paste-text":Guard(hwnd);System.Windows.Forms.Clipboard.SetText(args[4]);Hotkey(hwnd,"Ctrl+V",1,null);result=new{sent=true};break;
                    case "clipboard-text":Guard(hwnd);System.Windows.Forms.Clipboard.SetText(args[4]);result=new{staged=true};break;
                    case "read-control":result=ReadControl(pid,title,args[4]);break;
                    case "invoke-control":EchoUi.Invoke(pid,title,args[4]);result=new{invoked=true};break;
                    case "patterns":var patterns=new List<string>();foreach(var pattern in EchoUi.Find(pid,title,args[4]).GetSupportedPatterns())patterns.Add(pattern.ProgrammaticName);result=patterns;break;
                    case "range-snapshot":
                        var rangeElement=EchoUi.Find(pid,title,args[4]);object rawPattern;
                        if(!rangeElement.TryGetCurrentPattern(System.Windows.Automation.TextPattern.Pattern,out rawPattern))throw new InvalidOperationException("Owned input has no TextPattern");
                        var tp=(System.Windows.Automation.TextPattern)rawPattern;var selections=tp.GetSelection();
                        if(selections.Length!=1)throw new InvalidOperationException("Expected one test input selection");
                        var full=tp.DocumentRange;var head=full.Clone();var tail=full.Clone();
                        head.MoveEndpointByRange(System.Windows.Automation.Text.TextPatternRangeEndpoint.End,selections[0],System.Windows.Automation.Text.TextPatternRangeEndpoint.Start);
                        tail.MoveEndpointByRange(System.Windows.Automation.Text.TextPatternRangeEndpoint.Start,selections[0],System.Windows.Automation.Text.TextPatternRangeEndpoint.End);
                        result=new{document=full.GetText(4096),prefix=head.GetText(4096),selected=selections[0].GetText(4096),suffix=tail.GetText(4096),value=ReadControl(pid,title,args[4]),focus=rangeElement.Current.HasKeyboardFocus};break;
                    case "dump-tree":
                        var tree=new System.Text.StringBuilder();
                        var descendants=EchoUi.Root(pid,title).FindAll(System.Windows.Automation.TreeScope.Descendants,System.Windows.Automation.Condition.TrueCondition);
                        foreach(System.Windows.Automation.AutomationElement item in descendants) {
                            tree.Append(item.Current.ControlType.ProgrammaticName).Append(" | ").Append(item.Current.Name).Append(" | ").Append(item.Current.BoundingRectangle).AppendLine();
                        }
                        result=tree.ToString();break;
                    case "dump":result=EchoUi.Dump(pid,title);break;
                    case "ready":result=EchoUi.Ready(pid,title);break;
                    case "sample-actions":result=SampleActions(root,hwnd,pid,title,Int32.Parse(args[4]),args[5]);break;
                    case "record-window":result=RecordWindow(root,hwnd,pid,title,Int32.Parse(args[4]),args[5],args[6]);break;
                    case "sample-headers":result=SampleHeaders(root,hwnd,pid,title,Int32.Parse(args[4]),args[5]);break;
                    case "ime-metadata":result=ImeWindowMetadata(hwnd);break;
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
                    case "timed-hotkey":Hotkey(hwnd,args[4],1,null);result=new{sent=true,input_qpc=lastInputTimestamp,frequency=Stopwatch.Frequency};break;
                    case "paced-hotkey":PacedHotkey(hwnd,args[4],Int32.Parse(args[5]));result=new{sent=true};break;
                    case "hotkey-enter":Hotkey(hwnd,args[4],1,null);Thread.Sleep(Math.Max(0,Math.Min(500,Int32.Parse(args[5]))));OneKey(hwnd,13,1,false);result=new{sent=true};break;
                    case "hotkey-ready":result=HotkeyReady(root,hwnd,args[4],Int32.Parse(args[5]),args[6]);break;
                    case "measure-open":result=MeasureOpen(root,hwnd,args[4],Int32.Parse(args[5]));break;
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
