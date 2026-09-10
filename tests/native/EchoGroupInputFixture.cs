// Synthetic UIA TextPattern-only input. No clipboard capture or user data.
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Automation.Peers;
using System.Windows.Interop;
using System.Windows.Threading;
using System.Windows.Input;
using System.Globalization;

class GroupInput : TextBox {
    public string Role;
    protected override AutomationPeer OnCreateAutomationPeer() { return new GroupPeer(this); }
}
class GroupPeer : TextBoxAutomationPeer {
    readonly GroupInput input;
    public GroupPeer(GroupInput owner) : base(owner) { input = owner; }
    protected override AutomationControlType GetAutomationControlTypeCore() {
        return input.Role == "edit" ? AutomationControlType.Edit :
            input.Role == "document" ? AutomationControlType.Document : AutomationControlType.Group;
    }
    public override object GetPattern(PatternInterface pattern) {
        if (pattern == PatternInterface.Value || (pattern == PatternInterface.Text && input.Role == "no-text")) return null;
        return base.GetPattern(pattern);
    }
}
public static class EchoGroupInputFixture {
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint SendInput(uint count, Input[] inputs, int size);
    [StructLayout(LayoutKind.Sequential)] struct Key { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Explicit, Size = 40)] struct Input { [FieldOffset(0)] public uint type; [FieldOffset(8)] public Key key; }
    static void Keys(ushort vk, ushort scan, uint flags) {
        var pair = new[] { new Input { type=1, key=new Key { vk=vk, scan=scan, flags=flags } }, new Input { type=1, key=new Key { vk=vk, scan=scan, flags=flags|2 } } };
        if (SendInput(2, pair, Marshal.SizeOf(typeof(Input))) != 2) throw new InvalidOperationException("Owned key injection failed");
    }
    [STAThread] public static int Main(string[] args) {
        if (Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE") != "1" || args.Length != 1) return 2;
        string root = Path.GetFullPath(args[0]);
        var app = new Application();
        InputLanguageManager.Current.CurrentInputLanguage = CultureInfo.GetCultureInfo("en-US");
        var window = new Window { Title = "Echo synthetic Group input", Width = 600, Height = 240 };
        var panel = new StackPanel();
        GroupInput input = null;
        Action<string> reset = delegate(string role) {
            panel.Children.Clear();
            input = new GroupInput { Role = role, Text = "prefix ", AcceptsReturn = true, Height = 100, IsReadOnly = role == "readonly" };
            panel.Children.Add(input);
            panel.Children.Add(new Button { Content = "Non-input focus" });
            window.Activate(); SetForegroundWindow(new WindowInteropHelper(window).Handle);
            input.Focus(); input.CaretIndex = input.Text.Length;
            if (role == "inline") { input.Text = "prefix  suffix"; input.CaretIndex = 7; }
        };
        window.Content = panel;
        window.ContentRendered += delegate {
            reset("group");
            File.WriteAllText(Path.Combine(root, "ready"), new WindowInteropHelper(window).Handle.ToInt64().ToString());
        };
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(20) };
        timer.Tick += delegate {
            string path = Path.Combine(root, "command");
            if (!File.Exists(path)) return;
            string command = File.ReadAllText(path); File.Delete(path);
            if (command == "stop") { timer.Stop(); window.Close(); return; }
            if (command.StartsWith("type:") || command.StartsWith("key:")) {
                if (GetForegroundWindow() != new WindowInteropHelper(window).Handle || !input.IsKeyboardFocused) throw new InvalidOperationException("Owned input lost focus");
                if (command.StartsWith("type:")) foreach (char c in command.Substring(5)) Keys(0, c, 4);
                else Keys(UInt16.Parse(command.Substring(4)), 0, 0);
            } else if (command == "outside") input.Select(0, 2);
            else if (command == "blur") ((Button)panel.Children[1]).Focus();
            else if (command != "read" && command != "focused") reset(command);
            File.WriteAllText(Path.Combine(root, "response.tmp"), command == "focused" ? input.IsKeyboardFocused.ToString() : input.Text);
            File.Move(Path.Combine(root, "response.tmp"), Path.Combine(root, "response"));
        };
        timer.Start(); app.Run(window); return 0;
    }
}
