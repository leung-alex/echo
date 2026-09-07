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
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer { MaxJsonLength=1024*1024 };
    [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool AllowSetForegroundWindow(uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
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
    static void Atomic(string name,object value) {
        var path=Path.Combine(root,name);var temp=path+".tmp";
        File.WriteAllText(temp,Json.Serialize(value),new System.Text.UTF8Encoding(false));
        if(File.Exists(path))File.Delete(path);File.Move(temp,path);
    }
    static object State() {
        var fields=new Dictionary<string,object>();
        foreach(var pair in inputs) {
            var box=pair.Value;
            fields[pair.Key]=new {text=box.Text,start=box.SelectionStart,length=box.SelectionLength,focused=box.Focused,enter_count=enters[pair.Key],hwnd=box.Handle.ToInt64(),ime=Ime(box)};
        }
        return new {fields=fields,foreground=GetForegroundWindow()==form.Handle,deactivations=deactivations,pid=Process.GetCurrentProcess().Id};
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
            TextBoxBase edit=key=="rich"?(TextBoxBase)new RichTextBox():new TextBox();
            edit.AccessibleName="Inline fixture "+key;edit.Width=710;edit.Height=key=="rich"||key=="multiline"?75:30;
            edit.Font=new Font("Segoe UI",12);edit.Multiline=key=="rich"||key=="multiline";
            if(key=="password")((TextBox)edit).UseSystemPasswordChar=true;
            if(key=="readonly")edit.ReadOnly=true;
            edit.Text="pre| |post";edit.SelectionStart=4;edit.SelectionLength=0;
            inputs.Add(key,edit);enters.Add(key,0);panel.Controls.Add(edit);
            string captured=key;
            edit.KeyDown+=delegate(object sender,KeyEventArgs e){if(e.KeyCode==Keys.Enter){enters[captured]++;e.SuppressKeyPress=true;Atomic("native-state.json",State());}};
            edit.TextChanged+=delegate { if(form.IsHandleCreated)Atomic("native-state.json",State()); };
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
                if(op=="allow"){if(!AllowSetForegroundWindow(Convert.ToUInt32(request["pid"])))throw new InvalidOperationException("Foreground grant failed");}
                if(op=="move"){form.Location=new Point(Convert.ToInt32(request["x"]),Convert.ToInt32(request["y"]));}
                if(op=="ime-chinese")ChineseForOwnedInput(inputs[(string)request["control"]]);
                if(op=="ime-english")EnglishForOwnedInput(inputs[(string)request["control"]]);
                if(op=="reset"||op=="focus"||op=="selection") {
                    string key=(string)request["control"];TextBoxBase edit=inputs[key];
                    if(op=="reset") {edit.Text=(string)request["text"];enters[key]=0;}
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
