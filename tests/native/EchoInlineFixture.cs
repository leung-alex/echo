// Owned, synthetic Win32 input fixture. Never opens user documents or network connections.
using System;
using System.IO;
using System.Diagnostics;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using System.Drawing;
public static class EchoInlineFixture {
    static readonly Queue<object> focusEvents = new Queue<object>();
    static void RecordFocus(IntPtr window, Message message, uint sent) {
        while(focusEvents.Count >= 24) focusEvents.Dequeue();
        focusEvents.Enqueue(new { qpc=Stopwatch.GetTimestamp(), window=window.ToInt64(),
            message=message.Msg, other=message.WParam.ToInt64(), sent=sent,
            stack=message.Msg==8 ? new StackTrace().ToString() : "" });
    }
    sealed class OwnedTextBox : TextBox {
        public bool RejectSelection;
        public bool RefuseExternalReads;
        public int RefusedReadCount;
        public int AcquisitionDelayMs, AcquisitionDelayCount;
        public string SelectionFault = "";
        public int SelectionFaultCount;
        public string SelectionFaultError = "";
        public int PasteAttempts;
        public int RangeReplaceAttempts;
        public bool HoldClipboardDuringReplace;
        public bool ClipboardFaultHeld;
        public int ReadbackDelayMs, PendingReadbackDelayMs, ReadbackDelayCount;
        public bool LastPasteHadUnicode;
        public int PasteReplyDelayMs;
        public uint PasteSequenceBefore, PasteSequenceAfter;
        public long PasteOpenWindowBefore;
        public uint PasteOpenProcessBefore;
        public string PasteOpenImageBefore = "";
        public int PasteClipboardUnitsAfter = -1;
        [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
        [DllImport("user32.dll")] static extern uint InSendMessageEx(IntPtr reserved);
        [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
        [DllImport("user32.dll")] static extern IntPtr GetOpenClipboardWindow();
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
        [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool CloseClipboard();
        [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
        [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
        [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
        [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
        protected override void WndProc(ref Message message) {
            if(message.Msg==7 || message.Msg==8) RecordFocus(Handle, message, InSendMessageEx(IntPtr.Zero));
            if (message.Msg == 0x000E && AcquisitionDelayMs > 0 && InSendMessageEx(IntPtr.Zero) != 0) {
                int delay = AcquisitionDelayMs; AcquisitionDelayMs = 0;
                AcquisitionDelayCount++; System.Threading.Thread.Sleep(delay);
            }
            if (RefuseExternalReads && message.Msg == 0x000E && InSendMessageEx(IntPtr.Zero) != 0) {
                RefusedReadCount++; message.Result = new IntPtr(Int32.MaxValue); return;
            }
            if (message.Msg == 0x000E && PendingReadbackDelayMs > 0) {
                int delay = PendingReadbackDelayMs; PendingReadbackDelayMs = 0;
                ReadbackDelayCount++; System.Threading.Thread.Sleep(delay);
            }
            if (RejectSelection && message.Msg == 0x00B1) { message.Result = IntPtr.Zero; return; }
            bool replacement = message.Msg == 0x0302 || message.Msg == 0x00C2;
            if (message.Msg == 0x00C2) RangeReplaceAttempts++;
            if (message.Msg == 0x0302) {
                PasteAttempts++; LastPasteHadUnicode = IsClipboardFormatAvailable(13);
                PasteSequenceBefore = GetClipboardSequenceNumber();
                PasteOpenWindowBefore = GetOpenClipboardWindow().ToInt64();
                PasteOpenProcessBefore = 0; PasteOpenImageBefore = "";
                if (PasteOpenWindowBefore != 0) {
                    GetWindowThreadProcessId(new IntPtr(PasteOpenWindowBefore), out PasteOpenProcessBefore);
                }
            }
            if (replacement && HoldClipboardDuringReplace) {
                // Hold the clipboard on a separate thread only after Echo has
                // selected/validated the range and dispatched its single edit.
                // This deterministically models a competing clipboard reader.
                using (var ready = new System.Threading.ManualResetEvent(false))
                using (var release = new System.Threading.ManualResetEvent(false)) {
                    var worker = new System.Threading.Thread(delegate() {
                        ClipboardFaultHeld = OpenClipboard(IntPtr.Zero);
                        ready.Set();
                        if (ClipboardFaultHeld) {
                            try { release.WaitOne(1500); }
                            finally { CloseClipboard(); }
                        }
                    });
                    worker.IsBackground = true; worker.Start();
                    try {
                        if (!ready.WaitOne(500) || !ClipboardFaultHeld) { message.Result = IntPtr.Zero; return; }
                        base.WndProc(ref message);
                    } finally { release.Set(); worker.Join(); }
                }
            } else { base.WndProc(ref message); }
            if (message.Msg == 0x00B1 && SelectionFault.Length != 0) {
                string fault=SelectionFault;SelectionFault="";SelectionFaultCount++;
                try {
                    if(fault=="clipboard")Clipboard.SetText("echo-synthetic-competing-writer");
                    else if(fault=="focus")inputs["multiline"].Focus();
                } catch(Exception error) { SelectionFaultError=error.GetType().Name; }
            }
            if (replacement) PendingReadbackDelayMs = ReadbackDelayMs;
            if (message.Msg == 0x0302) {
                PasteSequenceAfter = GetClipboardSequenceNumber();
                if (PasteOpenProcessBefore != 0) {
                    try { using (var process = Process.GetProcessById((int)PasteOpenProcessBefore)) PasteOpenImageBefore = process.MainModule.FileName; }
                    catch { PasteOpenImageBefore = "unavailable"; }
                }
                PasteClipboardUnitsAfter = -1;
                if (OpenClipboard(Handle)) {
                    try {
                        var memory = GetClipboardData(13);
                        var pointer = memory == IntPtr.Zero ? IntPtr.Zero : GlobalLock(memory);
                        if (pointer != IntPtr.Zero) {
                            try {
                                int maximum = (int)Math.Min(GlobalSize(memory).ToUInt64() / 2, 8UL * 1024 * 1024);
                                for (int index = 0; index < maximum; index++)
                                    if (Marshal.ReadInt16(pointer, index * 2) == 0) { PasteClipboardUnitsAfter = index; break; }
                            }
                            finally { GlobalUnlock(memory); }
                        }
                    } finally { CloseClipboard(); }
                }
            }
            if (replacement && PasteReplyDelayMs > 0)
                System.Threading.Thread.Sleep(PasteReplyDelayMs);
        }
    }
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer { MaxJsonLength=1024*1024 };
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern IntPtr SetFocus(IntPtr window);
    [DllImport("imm32.dll")] static extern IntPtr ImmGetContext(IntPtr hwnd);
    [DllImport("imm32.dll")] static extern bool ImmReleaseContext(IntPtr hwnd,IntPtr context);
    [DllImport("imm32.dll")] static extern bool ImmGetOpenStatus(IntPtr context);
    [DllImport("imm32.dll")] static extern bool ImmSetOpenStatus(IntPtr context,bool open);
    [DllImport("imm32.dll")] static extern int ImmGetCompositionStringW(IntPtr context,uint index,IntPtr data,uint bytes);
    static object Ime(TextBoxBase input) {
        var context=ImmGetContext(input.Handle);
        if(context==IntPtr.Zero)return new {available=false,open=false,composition_bytes=-1};
        try{return new {available=true,open=ImmGetOpenStatus(context),composition_bytes=ImmGetCompositionStringW(context,8,IntPtr.Zero,0)};}
        finally{ImmReleaseContext(input.Handle,context);}
    }
    static void EnglishForOwnedInput(TextBoxBase input) {
        // Reset the control policy as well as the IMM context. A previous IME test
        // leaves ImeMode.On, which can reopen composition after the next focus event.
        input.ImeMode=ImeMode.Off;
        foreach(InputLanguage language in InputLanguage.InstalledInputLanguages) {
            if(language.Culture.TwoLetterISOLanguageName=="en") {
                InputLanguage.CurrentInputLanguage=language;
                break;
            }
        }
        var context=ImmGetContext(input.Handle);
        if(context!=IntPtr.Zero){try{ImmSetOpenStatus(context,false);}finally{ImmReleaseContext(input.Handle,context);}}
    }
    [DllImport("imm32.dll")] static extern bool ImmSetConversionStatus(IntPtr context,uint conversion,uint sentence);
    static void ChineseForOwnedInput(TextBoxBase input) {
        InputLanguage found=null;
        foreach(InputLanguage language in InputLanguage.InstalledInputLanguages)
            if(language.Culture.TwoLetterISOLanguageName=="zh"){found=language;break;}
        if(found==null)throw new InvalidOperationException("No installed Chinese input method");
        InputLanguage.CurrentInputLanguage=found; input.ImeMode=ImeMode.On;input.Focus();
        var context=ImmGetContext(input.Handle);
        if(context==IntPtr.Zero)throw new InvalidOperationException("Owned IME context unavailable");
        try{ImmSetOpenStatus(context,true);ImmSetConversionStatus(context,1,0);}
        finally{ImmReleaseContext(input.Handle,context);}
    }
    static string root;
    static Form form;
    static Dictionary<string,TextBoxBase> inputs=new Dictionary<string,TextBoxBase>();
    static Dictionary<string,int> enters=new Dictionary<string,int>();
    static int deactivations;
    static bool stateQueued;
    static void QueueState() {
        if (!form.IsHandleCreated || stateQueued) return;
        stateQueued = true;
        form.BeginInvoke((Action)delegate { stateQueued = false; Atomic("native-state.json", State()); });
    }
    static void Atomic(string name,object value) {
        var path=Path.Combine(root,name);var temp=path+".tmp";
        File.WriteAllText(temp,Json.Serialize(value),new System.Text.UTF8Encoding(false));
        // Readers can briefly hold a non-delete-sharing Windows handle. Never
        // delete the last complete report before the replacement is publishable.
        for (int attempt=0;;attempt++) {
            try { if(File.Exists(path))File.Replace(temp,path,null); else File.Move(temp,path); return; }
            catch(IOException) { if(attempt>=50)throw; System.Threading.Thread.Sleep(2); }
        }
    }
    static object State() {
        var fields=new Dictionary<string,object>();
        foreach(var pair in inputs) {
            var box=pair.Value;
            var owned = box as OwnedTextBox;
            fields[pair.Key]=new {text=box.Text,start=box.SelectionStart,length=box.SelectionLength,focused=box.Focused,enter_count=enters[pair.Key],hwnd=box.Handle.ToInt64(),ime=Ime(box),paste_attempts=owned==null?-1:owned.PasteAttempts,paste_had_unicode=owned!=null&&owned.LastPasteHadUnicode,
                paste_sequence_before=owned==null?0:owned.PasteSequenceBefore,paste_sequence_after=owned==null?0:owned.PasteSequenceAfter,
                paste_open_window_before=owned==null?0:owned.PasteOpenWindowBefore,paste_clipboard_units_after=owned==null?-1:owned.PasteClipboardUnitsAfter,
                paste_open_process_before=owned==null?0:owned.PasteOpenProcessBefore,paste_open_image_before=owned==null?"":owned.PasteOpenImageBefore,
                range_replace_attempts=owned==null?-1:owned.RangeReplaceAttempts,clipboard_fault_held=owned!=null&&owned.ClipboardFaultHeld,
                readback_delay_count=owned==null?0:owned.ReadbackDelayCount,
                refused_read_count=owned==null?0:owned.RefusedReadCount,
                acquisition_delay_count=owned==null?0:owned.AcquisitionDelayCount,
                selection_fault_count=owned==null?0:owned.SelectionFaultCount,
                selection_fault_error=owned==null?"":owned.SelectionFaultError};
        }
        IntPtr foregroundWindow=GetForegroundWindow();uint foregroundProcess;
        GetWindowThreadProcessId(foregroundWindow,out foregroundProcess);
        string foregroundName="",foregroundCreated="";
        try {using(var process=Process.GetProcessById((int)foregroundProcess)) {
            foregroundName=process.ProcessName;
            foregroundCreated=process.StartTime.ToUniversalTime().ToString("o");
        }} catch { }
        return new {focus_events=focusEvents.ToArray(),window=form.Handle.ToInt64(),fields=fields,foreground=foregroundWindow==form.Handle,
            foreground_hwnd=foregroundWindow.ToInt64(),foreground_pid=foregroundProcess,foreground_process=foregroundName,foreground_created_utc=foregroundCreated,
            deactivations=deactivations,pid=Process.GetCurrentProcess().Id};
    }
    [STAThread] public static int Main(string[] args) {
        if(args.Length!=2||Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE")!="1")return 2;
        root=Path.GetFullPath(args[0]);
        var marker=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(Path.Combine(root,"data","synthetic-fixture.json")));
        if(!Convert.ToBoolean(marker["synthetic"])||Convert.ToBoolean(marker["capture_enabled"]))return 3;
        SetProcessDpiAwarenessContext(new IntPtr(-4));Application.EnableVisualStyles();
        form=new Form{Text=args[1],Width=820,Height=590,StartPosition=FormStartPosition.Manual,Location=new Point(480,300)};
        var panel=new TableLayoutPanel{Dock=DockStyle.Fill,ColumnCount=1,RowCount=10,Padding=new Padding(22),AutoScroll=true};
        form.Controls.Add(panel);
        foreach(var key in new[]{"single","multiline","rich","password","readonly"}) {
            panel.Controls.Add(new Label{Text="Owned "+key+" input",AutoSize=true});
            TextBoxBase edit=key=="rich"?(TextBoxBase)new RichTextBox():new OwnedTextBox();
            edit.AccessibleName="Inline fixture "+key;edit.Width=710;edit.Height=key=="rich"||key=="multiline"?75:30;
            edit.Font=new Font("Segoe UI",12);edit.Multiline=key=="rich"||key=="multiline";
            if(key=="password")((TextBox)edit).UseSystemPasswordChar=true;
            if(key=="readonly")edit.ReadOnly=true;
            edit.Text="pre| |post";edit.SelectionStart=4;edit.SelectionLength=0;
            inputs.Add(key,edit);enters.Add(key,0);panel.Controls.Add(edit);
            string captured=key;
            edit.KeyDown+=delegate(object sender,KeyEventArgs e){if(e.KeyCode==Keys.Enter){enters[captured]++;e.SuppressKeyPress=true;QueueState();}};
            // Do not throw file-I/O errors in the middle of the native control's
            // WM_PASTE transaction (which may raise multiple TextChanged events).
            edit.TextChanged+=delegate { QueueState(); };
        }
        form.Deactivate+=delegate { deactivations++; };
        var timer=new Timer{Interval=20};
        timer.Tick+=delegate {
            var command=Path.Combine(root,"native-command.json");if(!File.Exists(command))return;
            Dictionary<string,object> request;
            try{request=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(command));File.Delete(command);}catch(IOException){return;}
            string id=(string)request["id"];string op=(string)request["op"];
            try {
                if(op=="quit"){timer.Stop();Atomic("native-response.json",new{id=id,status="PASS",value=State()});form.Close();return;}
                if(op=="clipboard-matches") {
                    bool matches=Clipboard.ContainsText() && Clipboard.GetText()==(string)request["expected"];
                    Atomic("native-response.json",new{id=id,status="PASS",value=new{matches=matches}});return;
                }
                if(op=="allow"){if(!AllowSetForegroundWindow(Convert.ToUInt32(request["pid"])))throw new InvalidOperationException("Foreground grant failed");}
                if(op=="move"){form.Location=new Point(Convert.ToInt32(request["x"]),Convert.ToInt32(request["y"]));}
                if(op=="root-focus-roundtrip") {
                    var edit=inputs["single"];
                    if(GetForegroundWindow()!=form.Handle || !edit.Focused)
                        throw new InvalidOperationException("Root-focus test requires its owned active input");
                    var restore=new Timer{Interval=40};
                    restore.Tick+=delegate {
                        restore.Stop(); restore.Dispose();
                        if(GetForegroundWindow()==form.Handle)edit.Focus();
                        QueueState();
                    };
                    SetFocus(form.Handle); restore.Start();
                }
                if(op=="ime-chinese")ChineseForOwnedInput(inputs[(string)request["control"]]);
                if(op=="ime-english")EnglishForOwnedInput(inputs[(string)request["control"]]);
                if(op=="selection-policy")((OwnedTextBox)inputs["single"]).RejectSelection=Convert.ToBoolean(request["reject"]);
                if(op=="read-refusal-policy")((OwnedTextBox)inputs["single"]).RefuseExternalReads=Convert.ToBoolean(request["enabled"]);
                if(op=="acquisition-delay-policy")((OwnedTextBox)inputs["single"]).AcquisitionDelayMs=Math.Max(0,Math.Min(500,Convert.ToInt32(request["delay_ms"])));
                if(op=="selection-fault-policy") {
                    string fault=(string)request["fault"];
                    if(fault!=""&&fault!="clipboard"&&fault!="focus")throw new ArgumentException("Unknown selection fault");
                    var box=(OwnedTextBox)inputs["single"];box.SelectionFault=fault;box.SelectionFaultCount=0;box.SelectionFaultError="";
                }
                if(op=="paste-reply-policy")((OwnedTextBox)inputs["single"]).PasteReplyDelayMs=Math.Max(0,Math.Min(1500,Convert.ToInt32(request["delay_ms"])));
                if(op=="clipboard-contention-policy")((OwnedTextBox)inputs["single"]).HoldClipboardDuringReplace=Convert.ToBoolean(request["enabled"]);
                if(op=="readback-delay-policy")((OwnedTextBox)inputs["single"]).ReadbackDelayMs=Math.Max(0,Math.Min(1500,Convert.ToInt32(request["delay_ms"])));
                if(op=="reset"||op=="focus"||op=="selection") {
                    string key=(string)request["control"];TextBoxBase edit=inputs[key];
                    if(op=="reset") {edit.Text=(string)request["text"];enters[key]=0;var owned=edit as OwnedTextBox;if(owned!=null){owned.PasteAttempts=0;owned.LastPasteHadUnicode=false;owned.RangeReplaceAttempts=0;owned.ClipboardFaultHeld=false;owned.ReadbackDelayCount=0;owned.PendingReadbackDelayMs=0;}}
                    if(op!="selection") {SetForegroundWindow(form.Handle);form.Activate();edit.Focus();}
                    if(request.ContainsKey("start"))edit.Select(Convert.ToInt32(request["start"]),Convert.ToInt32(request["length"]));
                    if(op=="reset"){EnglishForOwnedInput(edit);deactivations=0;}
                }
                var value=State();Atomic("native-state.json",value);Atomic("native-response.json",new{id=id,status="PASS",value=value});
            } catch(Exception error){Atomic("native-response.json",new{id=id,status="FAIL",error=error.ToString()});}
        };
        form.Shown+=delegate {inputs["single"].Focus();Atomic("native-ready.json",new{pid=Process.GetCurrentProcess().Id,title=form.Text});};
        timer.Start();Application.Run(form);return 0;
    }
}
