// Explicitly authorized integration input; separate from Echo's no-clipboard test bridge.
// Every input targets the PID/title of a harness-owned process and verifies foreground first.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Web.Script.Serialization;
public static class EchoInlineDriver {
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
    static void ClickOwned(int pid,string title,string label,IntPtr hwnd,bool activate) {
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
        try {
            if(!SetCursorPos(point.X,point.Y)||GetAncestor(WindowFromPoint(point),2)!=hwnd)throw new InvalidOperationException("Owned pointer target changed before click");
            var list=new List<Input>{new Input{Type=0,Data=new InputData{Mouse=new Mouse{Flags=2}}},new Input{Type=0,Data=new InputData{Mouse=new Mouse{Flags=4}}}};
            Send(list);
            var watch=Stopwatch.StartNew();while(activate&&GetForegroundWindow()!=hwnd&&watch.ElapsedMilliseconds<1500)Thread.Sleep(10);
            if(activate&&GetForegroundWindow()!=hwnd)throw new InvalidOperationException("Physical click did not activate the owned fixture");
        } finally {Point current;GetCursorPos(out current);if(current.X==point.X&&current.Y==point.Y)SetCursorPos(old.X,old.Y);if(activate)SetWindowPos(hwnd,new IntPtr(-2),0,0,0,0,0x0001|0x0002|0x0010);}
    }
    static void Guard(IntPtr hwnd) {
        if(GetForegroundWindow()!=hwnd)throw new InvalidOperationException("Owned target lost foreground; input was not sent");
        foreach(var vk in new[]{0x10,0x11,0x12,0x5b,0x5c})if((GetAsyncKeyState(vk)&0x8000)!=0)throw new InvalidOperationException("A physical modifier is held; input cancelled");
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
        var icon=EchoUi.Find(pid,title,"Copy item").Current.BoundingRectangle;
        Rect initial;GetWindowRect(echo,out initial);
        int ox=(int)(icon.Left-initial.Left),oy=(int)(icon.Top-initial.Top);
        var frames=new List<object>();var watch=Stopwatch.StartNew();
        using(var bitmap=new System.Drawing.Bitmap((int)icon.Width,(int)icon.Height))
        using(var graphics=System.Drawing.Graphics.FromImage(bitmap)) {
            File.WriteAllText(Path.Combine(root,"actions.ready"),"owned copy button samples only");
            while(watch.ElapsedMilliseconds<12000&&!File.Exists(Path.Combine(root,"actions.stop"))) {
                if(GetForegroundWindow()!=input||EchoUi.Window(pid,title,true)!=echo)throw new InvalidOperationException("Action sample lost target focus");
                Rect rect;GetWindowRect(echo,out rect);
                graphics.CopyFromScreen(rect.Left+ox,rect.Top+oy,0,0,bitmap.Size,System.Drawing.CopyPixelOperation.SourceCopy);
                var colors=new List<int>();
                for(int y=7;y<bitmap.Height-7;y+=2)for(int x=7;x<bitmap.Width-7;x+=2)colors.Add(bitmap.GetPixel(x,y).ToArgb());
                frames.Add(new{ms=watch.ElapsedMilliseconds,colors=colors});Thread.Sleep(10);
            }
        }
        return new{source="physical pixels within owned Echo Copy action",frames=frames};
    }
    delegate bool EnumWindowProc(IntPtr window,IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowProc callback,IntPtr parameter);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr window,System.Text.StringBuilder name,int maximum);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr window,uint command);
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
                switch(args[0]) {
                    case "activate-owned":ClickOwned(pid,title,args[4],hwnd,true);result=Geometry(hwnd);break;
                    case "click-owned":ClickOwned(pid,title,args[4],hwnd,false);result=Geometry(hwnd);break;
                    case "english-owned":result=EnglishOwned(hwnd);break;
                    case "close-owned":if(!PostMessageW(hwnd,0x0010,IntPtr.Zero,IntPtr.Zero))throw new InvalidOperationException("Owned close message failed");result=new{requested=true};break;
                    case "text":Text(hwnd,args[4],false);result=new{sent=true};break;
                    case "text-enter":Text(hwnd,args[4],true);result=new{sent=true};break;
                    case "key":OneKey(hwnd,UInt32.Parse(args[4]),args.Length>5?Int32.Parse(args[5]):1,args.Length>6&&args[6]=="shift");result=new{sent=true};break;
                    case "held-enter":HoldEnter(hwnd);result=new{sent=true};break;
                    case "paste-text":Guard(hwnd);System.Windows.Forms.Clipboard.SetText(args[4]);Hotkey(hwnd,"Ctrl+V",1,null);result=new{sent=true};break;
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
                    case "sample-actions":result=SampleActions(root,hwnd,pid,title,Int32.Parse(args[4]),args[5]);break;
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
