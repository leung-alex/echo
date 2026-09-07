// Isolated UIA-only WPF target. No HWND edit control and no user data.
using System;
using System.IO;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Web.Script.Serialization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Automation;
using System.Windows.Interop;
using System.Windows.Threading;
public static class EchoWpfCaretFixture {
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool DestroyCaret();
    [STAThread] public static int Main(string[] args) {
        if(Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE")!="1" || args.Length!=2) return 2;
        string root=Path.GetFullPath(args[0]);
        var json=new JavaScriptSerializer();
        var marker=json.Deserialize<System.Collections.Generic.Dictionary<string,object>>(File.ReadAllText(Path.Combine(root,"data","synthetic-fixture.json")));
        if(!Convert.ToBoolean(marker["synthetic"])||Convert.ToBoolean(marker["capture_enabled"])) return 3;
        var app=new Application();
        var window=new Window { Title=args[1],Width=620,Height=320,Left=680,Top=120,WindowStartupLocation=WindowStartupLocation.Manual };
        var panel=new StackPanel { Margin=new Thickness(24) };
        panel.Children.Add(new TextBlock { Text="Echo isolated UI Automation input",FontSize=18,Margin=new Thickness(0,0,0,16) });
        var edit=new TextBox { Text="uia-fixture-prefix ",FontSize=18,AcceptsReturn=true,Height=130,TextWrapping=TextWrapping.Wrap };
        AutomationProperties.SetName(edit,"Owned UIA text input");panel.Children.Add(edit);window.Content=panel;
        var timer=new DispatcherTimer { Interval=TimeSpan.FromMilliseconds(20) };
        Action writeGeometry=delegate {
            var caret=edit.GetRectFromCharacterIndex(Math.Max(0,edit.CaretIndex-1),true);
            var point=edit.PointToScreen(new Point(caret.Right,caret.Top));
            var scale=PresentationSource.FromVisual(edit).CompositionTarget.TransformToDevice;
            var state=new {pid=Process.GetCurrentProcess().Id,title=window.Title,text=edit.Text,caret=new[]{point.X,point.Y,point.X+1,point.Y+caret.Height*scale.M22},focused=edit.IsKeyboardFocused};
            string temp=Path.Combine(root,"wpf.state.tmp");File.WriteAllText(temp,json.Serialize(state));
            string path=Path.Combine(root,"wpf.state.json");if(File.Exists(path))File.Delete(path);File.Move(temp,path);
        };
        window.ContentRendered+=delegate { edit.Focus();edit.CaretIndex=edit.Text.Length;writeGeometry();File.WriteAllText(Path.Combine(root,"wpf.ready"),Process.GetCurrentProcess().Id.ToString()); };
        timer.Tick+=delegate {
            if(File.Exists(Path.Combine(root,"wpf.stop"))){timer.Stop();window.Close();return;}
            string command=Path.Combine(root,"wpf.command");
            if(File.Exists(command)) {
                string op=File.ReadAllText(command);File.Delete(command);
                if(op=="focus") { SetForegroundWindow(new WindowInteropHelper(window).Handle);edit.Focus();edit.CaretIndex=edit.Text.Length; }
                if(op=="readonly") { edit.IsReadOnly=true;edit.Focus(); }
                // Explicitly exercise accessibility geometry without a Win32 caret.
                if(op=="suppress-native-caret") { DestroyCaret(); File.WriteAllText(Path.Combine(root,"wpf.no-native-caret"),"suppressed by owned fixture"); }
                writeGeometry();
            }
        };
        timer.Start();app.Run(window);return 0;
    }
}
