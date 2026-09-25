// Owned synthetic WPF editor used by the caret geometry acceptance runner.
//
// This is a real WPF/TSF target. It never receives text from the runner and
// never exposes document contents in its oracle. RealTSF scenarios remain
// separate from the explicit MockCOM fault adapter below.
using System;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;

public static class CaretTsfFixture {
    static string Root;
    static TextBox Editor;
    static StackPanel Panel;
    static Window Window;
    static string Mode = "normal";
    static string Scenario = "normal";
    static bool Stop;
    static int ScenarioCaret;

    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
    struct NativeRect { public int Left, Top, Right, Bottom; }

    [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true)]
    static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    static extern uint GetDpiForWindow(IntPtr hwnd);

    [STAThread]
    public static int Main(string[] args) {
        if (args.Length != 1 || Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE") != "1") return 2;
        Root = Path.GetFullPath(args[0]);
        Directory.CreateDirectory(Root);
        WriteFault("clear");
        var app = new Application();
        Window = new Window {
            Width = 720, Height = 360, Left = 620, Top = 120,
            WindowStartupLocation = WindowStartupLocation.Manual,
            Title = "Echo owned caret fixture"
        };
        Panel = new StackPanel { Margin = new Thickness(24) };
        Panel.Children.Add(new TextBlock {
            Text = "Owned synthetic TSF editor", FontSize = 18,
            Margin = new Thickness(0, 0, 0, 12)
        });
        CreateEditor("synthetic caret fixture text");
        Window.Content = Panel;
        Window.ContentRendered += delegate {
            Editor.Focus(); Editor.CaretIndex = Editor.Text.Length;
            WriteReady(); WriteOracle();
        };
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(20) };
        timer.Tick += delegate { if (Stop) { timer.Stop(); Window.Close(); } else WriteOracle(); };
        timer.Start();
        var commandThread = new Thread(CommandLoop) { IsBackground = true, Name = "caret-fixture-command" };
        commandThread.Start();
        app.Run(Window);
        Stop = true;
        return 0;
    }

    static void CreateEditor(string text) {
        Editor = new TextBox {
            Width = 620, Height = 180, FontSize = 18,
            Text = text, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
            InputScope = new InputScope {
                // WPF's enum omits the TSF IS_TEXT value; retaining the
                // contract value here makes the owned fixture explicitly
                // general text instead of leaving sensitivity unknown.
                Names = { new InputScopeName((InputScopeNameValue)57) }
            }
        };
        Panel.Children.Add(Editor);
    }

    static void ReplaceEditor(string text) {
        if (Editor != null) Panel.Children.Remove(Editor);
        CreateEditor(text);
        Editor.Focus();
        Editor.CaretIndex = Editor.Text.Length;
    }

    static void WriteReady() {
        var handle = new WindowInteropHelper(Window).Handle;
        File.WriteAllText(Path.Combine(Root, "fixture.ready.json"),
            "{\"pid\":" + Process.GetCurrentProcess().Id +
            ",\"hwnd\":" + handle.ToInt64() +
            ",\"pipe\":\"EchoCaretFixture-" + Process.GetCurrentProcess().Id +
            "\",\"command\":\"fixture.command.json\",\"response\":\"fixture.response.json\"}");
    }

    static bool TryCaretRect(out NativeRect windowRect, out int[] caret, out uint dpi) {
        windowRect = new NativeRect(); caret = null; dpi = 96;
        if (Editor == null || Window == null || !Window.IsVisible) return false;
        var source = PresentationSource.FromVisual(Editor);
        var scale = source == null ? new Vector(1, 1) : source.CompositionTarget.TransformToDevice.Transform(new Vector(1, 1));
        var index = Math.Max(0, Math.Min(Editor.Text.Length, Editor.CaretIndex));
        Rect r;
        try {
            if (Editor.Text.Length == 0) {
                r = new Rect(new Point(0, 0), new Size(1, Math.Max(1, Editor.FontSize * 1.35)));
            } else if (index == 0) {
                r = Editor.GetRectFromCharacterIndex(0, false);
            } else {
                r = Editor.GetRectFromCharacterIndex(index - 1, true);
            }
            var p = Editor.PointToScreen(new Point(r.Left, r.Top));
            var q = Editor.PointToScreen(new Point(r.Right, r.Bottom));
            var left = (int)Math.Round(p.X * scale.X);
            var top = (int)Math.Round(p.Y * scale.Y);
            var right = Math.Max(left + 1, (int)Math.Round(q.X * scale.X));
            var bottom = Math.Max(top + 1, (int)Math.Round(q.Y * scale.Y));
            caret = new[] { left, top, right, bottom };
        } catch { return false; }
        var hwnd = new WindowInteropHelper(Window).Handle;
        if (!GetWindowRect(hwnd, out windowRect)) return false;
        dpi = GetDpiForWindow(hwnd);
        if (dpi == 0) dpi = (uint)Math.Max(1, Math.Round(96 * scale.X));
        return caret[2] > caret[0] && caret[3] > caret[1];
    }

    static void WriteOracle() {
        NativeRect windowRect; int[] caret; uint dpi;
        if (!TryCaretRect(out windowRect, out caret, out dpi)) return;
        var handle = new WindowInteropHelper(Window).Handle;
        var line = "{\"pid\":" + Process.GetCurrentProcess().Id +
            ",\"hwnd\":" + handle.ToInt64() +
            ",\"input_hwnd\":" + handle.ToInt64() +
            ",\"visible\":true,\"dpi\":" + dpi +
            ",\"window_rect\":[" + windowRect.Left + "," + windowRect.Top + "," +
            (windowRect.Right - windowRect.Left) + "," + (windowRect.Bottom - windowRect.Top) +
            "],\"caret\":[" + caret[0] + "," + caret[1] + "," + caret[2] + "," + caret[3] +
            "],\"mode\":\"" + Mode + "\",\"scenario\":\"" + Scenario +
            "\",\"caret_index\":" + Editor.CaretIndex +
            ",\"sequence\":" + (++ScenarioCaret) + "}";
        var tmp = Path.Combine(Root, "fixture.oracle.tmp");
        File.WriteAllText(tmp, line, Encoding.UTF8);
        var path = Path.Combine(Root, "fixture.oracle.json");
        for (var attempt = 0; attempt < 40; attempt++) {
            try {
                if (File.Exists(path)) File.Replace(tmp, path, null);
                else File.Move(tmp, path);
                return;
            } catch (IOException) {
                Thread.Sleep(2);
            }
        }
        try { if (File.Exists(tmp)) File.Delete(tmp); } catch { }
    }

    static bool KnownFault(string value) {
        switch (value) {
            case "clear": case "missing-uia": case "password":
            case "unknown-sensitivity": case "no-layout": case "invalid-rect":
            case "different-view": case "focus-race": case "request-error":
            case "late-result": case "never-delivered": case "late-close":
            case "selection": case "reentrancy": case "source-conflict":
            case "protocol": case "rollover": case "reuse": case "release-cap":
            case "lifecycle-never-delivered": case "lifecycle-late-close":
            case "lifecycle-reentrancy-close": case "lifecycle-release-cap":
                return true;
            default: return false;
        }
    }

    static void WriteFault(string value) {
        if (!KnownFault(value)) throw new InvalidOperationException("unknown fault");
        var path = Path.Combine(Root, "caret-fault.txt");
        var tmp = path + ".tmp";
        File.WriteAllText(tmp, value, Encoding.ASCII);
        for (var attempt = 0; attempt < 40; attempt++) {
            try {
                if (File.Exists(path)) File.Replace(tmp, path, null);
                else File.Move(tmp, path);
                return;
            } catch (IOException) {
                Thread.Sleep(2);
            }
        }
        try { if (File.Exists(tmp)) File.Delete(tmp); } catch { }
        throw new IOException("fault adapter file was busy");
    }

    static void CommandLoop() {
        var commandPath = Path.Combine(Root, "fixture.command.json");
        var responsePath = Path.Combine(Root, "fixture.response.json");
        while (!Stop) {
            try {
                if (!File.Exists(commandPath)) { Thread.Sleep(10); continue; }
                var line = File.ReadAllText(commandPath, Encoding.UTF8);
                File.Delete(commandPath);
                var split = line.IndexOf('|');
                var nonce = split > 0 ? line.Substring(0, split) : "";
                var command = split > 0 ? line.Substring(split + 1) : line;
                var result = Dispatch(command);
                var escaped = command.Replace("\\", "\\\\").Replace("\"", "\\\"");
                var response = "{\"nonce\":\"" + nonce + "\",\"command\":\"" +
                    escaped + "\"," + result + "}";
                var temp = responsePath + ".tmp";
                File.WriteAllText(temp, response, Encoding.UTF8);
                if (File.Exists(responsePath)) File.Delete(responsePath);
                File.Move(temp, responsePath);
            } catch { Thread.Sleep(20); }
        }
    }

    static string Pass() { return "\"status\":\"PASS\""; }
    static string Unsupported(string reason) {
        return "\"status\":\"UNSUPPORTED\",\"reason\":\"" + reason + "\"";
    }
    static string Fail(string reason) {
        return "\"status\":\"FAIL\",\"reason\":\"" + reason + "\"";
    }

    static string Dispatch(string command) {
        try {
            var result = Pass();
            Window.Dispatcher.Invoke(DispatcherPriority.Send, new Action(delegate {
                if (command.StartsWith("MoveCaret|")) {
                    int value;
                    if (!int.TryParse(command.Substring(10), out value)) throw new InvalidOperationException("invalid caret index");
                    Editor.CaretIndex = Math.Max(0, Math.Min(Editor.Text.Length, value)); Editor.Focus();
                } else if (command.StartsWith("CreateScenario|")) {
                    var value = command.Substring(15);
                    Scenario = value;
                    Mode = value;
                    Editor.IsReadOnly = false;
                    if (value == "normal") ReplaceEditor("synthetic caret fixture text");
                    else if (value == "empty") { Editor.Text = ""; Editor.CaretIndex = 0; Editor.Focus(); }
                    else if (value == "multiline") ReplaceEditor("line one\r\nline two\r\nline three");
                    else if (value == "context-replaced") ReplaceEditor("replacement TSF context");
                    else if (value == "readonly") { Editor.IsReadOnly = true; Editor.Focus(); }
                    else if (value == "mixed-dpi") {
                        result = Unsupported("mixed-dpi requires an explicit multi-monitor DPI layout");
                    }
                    else result = Unsupported("scenario requires a real COM fault adapter");
                } else if (command == "FocusEditor") {
                    Editor.Focus();
                } else if (command == "SetFault|readonly") {
                    Editor.IsReadOnly = true;
                } else if (command.StartsWith("Fault|")) {
                    WriteFault(command.Substring(6));
                } else if (command == "ShutdownOwnedFixture") {
                    Stop = true;
                } else if (command == "SetSyntheticModeForFixture|normal") {
                    Mode = "normal";
                } else if (command == "SetSyntheticModeForFixture|empty") {
                    Mode = "empty";
                } else {
                    result = Unsupported("command has no owned implementation");
                }
                WriteOracle();
            }));
            return result;
        } catch (Exception e) { return Fail(e.GetType().Name); }
    }
}
