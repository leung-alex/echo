// Owned synthetic WPF editor used by the caret geometry acceptance runner.
//
// This is a real WPF/TSF target. It never receives text from the runner and
// never exposes document contents in its oracle. RealTSF scenarios remain
// separate from the explicit MockCOM fault adapter below.
using System;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
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
    static NativeEditorHost Editor;
    static NativeEditorHost EditorOne;
    static NativeEditorHost EditorTwo;
    static StackPanel Panel;
    static Window Window;
    static string Mode = "normal";
    static string Scenario = "normal";
    static bool Stop;
    static int ScenarioCaret;

    const uint WS_CHILD = 0x40000000;
    const uint WS_VISIBLE = 0x10000000;
    const uint WS_TABSTOP = 0x00010000;
    const uint WS_VSCROLL = 0x00200000;
    const uint WS_EX_CLIENTEDGE = 0x00000200;
    const uint ES_MULTILINE = 0x0004;
    const uint ES_AUTOVSCROLL = 0x0040;
    const uint ES_WANTRETURN = 0x1000;
    const uint ES_NOHIDESEL = 0x0100;
    const uint WM_USER = 0x0400;
    const uint EM_SETSEL = 0x00B1;
    const uint EM_GETSEL = 0x00B0;
    const uint EM_SETREADONLY = 0x00CF;
    const uint EM_SETLIMITTEXT = WM_USER + 53;
    const uint WM_SETFONT = 0x0030;
    const uint MONITOR_DEFAULTTONEAREST = 2;
    const uint MDT_EFFECTIVE_DPI = 0;
    static readonly IntPtr DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = new IntPtr(-4);

    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
    struct NativeRect { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    struct NativePoint { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    struct MonitorInfo {
        public int CbSize;
        public NativeRect RcMonitor;
        public NativeRect RcWork;
        public uint Flags;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct GuiThreadInfo {
        public int CbSize;
        public uint Flags;
        public IntPtr HwndActive;
        public IntPtr HwndFocus;
        public IntPtr HwndCapture;
        public IntPtr HwndMenuOwner;
        public IntPtr HwndMoveSize;
        public IntPtr HwndCaret;
        public NativeRect RcCaret;
    }

    [System.Runtime.InteropServices.DllImport("user32.dll", SetLastError = true)]
    static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr CreateWindowExW(uint exStyle, string className, string windowName,
        uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu,
        IntPtr instance, IntPtr param);

    [DllImport("user32.dll", SetLastError = true)]
    static extern bool DestroyWindow(IntPtr hwnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr LoadCursorW(IntPtr instance, IntPtr cursor);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr LoadLibraryW(string fileName);

    [DllImport("user32.dll")]
    static extern IntPtr SendMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern bool SetWindowTextW(IntPtr hwnd, string text);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetWindowTextLengthW(IntPtr hwnd);

    [DllImport("user32.dll")]
    static extern bool SetFocus(IntPtr hwnd);

    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr hwnd, IntPtr processId);

    [DllImport("user32.dll")]
    static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);

    [DllImport("user32.dll")]
    static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    [DllImport("user32.dll")]
    static extern bool ClientToScreen(IntPtr hwnd, ref NativePoint point);

    [DllImport("user32.dll")]
    static extern bool LogicalToPhysicalPointForPerMonitorDPI(IntPtr hwnd, ref NativePoint point);

    [DllImport("user32.dll")]
    static extern bool GetGUIThreadInfo(uint threadId, ref GuiThreadInfo info);

    [DllImport("kernel32.dll")]
    static extern uint GetCurrentThreadId();

    [DllImport("user32.dll")]
    static extern IntPtr MonitorFromRect(ref NativeRect rect, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern bool GetMonitorInfoW(IntPtr monitor, ref MonitorInfo info);

    [DllImport("shcore.dll")]
    static extern int GetDpiForMonitor(IntPtr monitor, uint type, out uint dpiX, out uint dpiY);

    [DllImport("user32.dll")]
    static extern IntPtr GetWindowLongPtrW(IntPtr hwnd, int index);

    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr hwnd);

    sealed class NativeEditorHost : HwndHost {
        public new IntPtr Handle { get; private set; }
        public string InitialText { get; private set; }

        public NativeEditorHost(string text) {
            InitialText = text ?? "";
            Width = 620;
            Height = 180;
            Focusable = true;
        }

        public int TextLength {
            get {
                return Handle == IntPtr.Zero ? InitialText.Length : Math.Max(0, GetWindowTextLengthW(Handle));
            }
        }

        public int CaretIndex {
            get {
                if (Handle == IntPtr.Zero) return InitialText.Length;
                var start = Marshal.AllocHGlobal(sizeof(int));
                var end = Marshal.AllocHGlobal(sizeof(int));
                try {
                    SendMessageW(Handle, EM_GETSEL, start, end);
                    return Marshal.ReadInt32(end);
                } finally {
                    Marshal.FreeHGlobal(start);
                    Marshal.FreeHGlobal(end);
                }
            }
            set {
                if (Handle == IntPtr.Zero) return;
                var index = Math.Max(0, value);
                SendMessageW(Handle, EM_SETSEL, (IntPtr)index, (IntPtr)index);
            }
        }

        public bool IsReadOnly {
            set {
                if (Handle != IntPtr.Zero)
                    SendMessageW(Handle, EM_SETREADONLY, value ? new IntPtr(1) : IntPtr.Zero, IntPtr.Zero);
            }
        }

        public void SetText(string text) {
            InitialText = text ?? "";
            if (Handle != IntPtr.Zero) {
                SetWindowTextW(Handle, InitialText);
                CaretIndex = InitialText.Length;
            }
        }

        public void FocusEditor() {
            Focus();
            if (Handle != IntPtr.Zero) SetFocus(Handle);
        }

        protected override HandleRef BuildWindowCore(HandleRef hwndParent) {
            // RichEdit is a native TSF-aware child.  Loading Msftedit here
            // keeps the host window and both switchable input HWNDs in the
            // same UI thread/process while leaving the WPF root as owner.
            LoadLibraryW("Msftedit.dll");
            Handle = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                "RICHEDIT50W",
                InitialText,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL |
                    ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | ES_NOHIDESEL,
                0, 0, 620, 180,
                hwndParent.Handle, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
            if (Handle == IntPtr.Zero) throw new InvalidOperationException("RichEdit child creation failed");
            SendMessageW(Handle, EM_SETLIMITTEXT, (IntPtr)4096, IntPtr.Zero);
            CaretIndex = InitialText.Length;
            return new HandleRef(this, Handle);
        }

        protected override void DestroyWindowCore(HandleRef hwnd) {
            if (hwnd.Handle != IntPtr.Zero) DestroyWindow(hwnd.Handle);
            Handle = IntPtr.Zero;
        }
    }

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
            Editor.FocusEditor(); Editor.CaretIndex = Editor.TextLength;
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
        EditorOne = new NativeEditorHost(text);
        EditorTwo = new NativeEditorHost(text);
        EditorTwo.Visibility = Visibility.Hidden;
        Panel.Children.Add(EditorOne);
        Panel.Children.Add(EditorTwo);
        Editor = EditorOne;
    }

    static void ReplaceEditor(string text) {
        if (Editor == null) return;
        Editor.SetText(text);
        Editor.IsReadOnly = false;
        Editor.FocusEditor();
        Editor.CaretIndex = Editor.TextLength;
    }

    static void SwitchEditor(bool second) {
        if (EditorOne == null || EditorTwo == null) return;
        EditorOne.Visibility = second ? Visibility.Hidden : Visibility.Visible;
        EditorTwo.Visibility = second ? Visibility.Visible : Visibility.Hidden;
        Editor = second ? EditorTwo : EditorOne;
        Editor.FocusEditor();
        Editor.CaretIndex = Editor.TextLength;
    }

    static void WriteReady() {
        var handle = new WindowInteropHelper(Window).Handle;
        File.WriteAllText(Path.Combine(Root, "fixture.ready.json"),
            "{\"pid\":" + Process.GetCurrentProcess().Id +
            ",\"hwnd\":" + handle.ToInt64() +
            ",\"pipe\":\"EchoCaretFixture-" + Process.GetCurrentProcess().Id +
            "\",\"command\":\"fixture.command.json\",\"response\":\"fixture.response.json\"}");
    }

    static bool TryCaretRect(out NativeRect windowRect, out NativeRect inputRect,
        out NativeRect workArea, out int[] caret, out uint dpi) {
        windowRect = new NativeRect(); inputRect = new NativeRect();
        workArea = new NativeRect(); caret = null; dpi = 0;
        if (Editor == null || Editor.Handle == IntPtr.Zero || Window == null || !Window.IsVisible)
            return false;
        var input = Editor.Handle;
        if (!IsWindowVisible(input)) return false;

        // GUITHREADINFO is the native caret oracle.  Its rcCaret is in the
        // focused child window's logical coordinates; convert each endpoint
        // exactly once through that child DPI context.  PointToScreen values
        // are already screen coordinates and must never be scaled again.
        var gui = new GuiThreadInfo { CbSize = Marshal.SizeOf(typeof(GuiThreadInfo)) };
        if (!GetGUIThreadInfo(GetCurrentThreadId(), ref gui)
            || gui.HwndFocus != input || gui.HwndCaret != input
            || gui.RcCaret.Right <= gui.RcCaret.Left
            || gui.RcCaret.Bottom <= gui.RcCaret.Top)
            return false;
        var a = new NativePoint { X = gui.RcCaret.Left, Y = gui.RcCaret.Top };
        var b = new NativePoint { X = gui.RcCaret.Right, Y = gui.RcCaret.Bottom };
        var targetContext = GetWindowDpiAwarenessContext(input);
        if (targetContext == IntPtr.Zero) return false;
        var previous = SetThreadDpiAwarenessContext(targetContext);
        try {
            if (!ClientToScreen(input, ref a) || !ClientToScreen(input, ref b)
                || !LogicalToPhysicalPointForPerMonitorDPI(input, ref a)
                || !LogicalToPhysicalPointForPerMonitorDPI(input, ref b))
                return false;
        } finally {
            SetThreadDpiAwarenessContext(previous);
        }
        caret = new[] { a.X, a.Y, Math.Max(a.X + 1, b.X), Math.Max(a.Y + 1, b.Y) };
        if (caret[2] <= caret[0] || caret[3] <= caret[1]) return false;

        var caretRect = new NativeRect { Left = caret[0], Top = caret[1], Right = caret[2], Bottom = caret[3] };
        var root = new WindowInteropHelper(Window).Handle;
        var pmPrevious = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        try {
            // All following APIs consume the fresh physical caret rectangle;
            // this selects the caret's monitor even when the root spans two.
            if (!GetWindowRect(root, out windowRect) || !GetWindowRect(input, out inputRect)) return false;
            var monitor = MonitorFromRect(ref caretRect, MONITOR_DEFAULTTONEAREST);
            if (monitor == IntPtr.Zero) return false;
            var info = new MonitorInfo { CbSize = Marshal.SizeOf(typeof(MonitorInfo)) };
            if (!GetMonitorInfoW(monitor, ref info)) return false;
            uint dpiX, dpiY;
            if (GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, out dpiX, out dpiY) != 0
                || dpiX < 48 || dpiX > 768)
                return false;
            workArea = info.RcWork;
            dpi = dpiX;
        } finally {
            SetThreadDpiAwarenessContext(pmPrevious);
        }
        return true;
    }

    static void WriteOracle() {
        NativeRect windowRect, inputRect, workArea; int[] caret; uint dpi;
        if (!TryCaretRect(out windowRect, out inputRect, out workArea, out caret, out dpi)) return;
        var handle = new WindowInteropHelper(Window).Handle;
        var input = Editor.Handle;
        var line = "{\"pid\":" + Process.GetCurrentProcess().Id +
            ",\"hwnd\":" + handle.ToInt64() +
            ",\"input_hwnd\":" + input.ToInt64() +
            ",\"visible\":true,\"dpi\":" + dpi +
            ",\"window_rect\":[" + windowRect.Left + "," + windowRect.Top + "," +
            (windowRect.Right - windowRect.Left) + "," + (windowRect.Bottom - windowRect.Top) +
            "],\"input_rect\":[" + inputRect.Left + "," + inputRect.Top + "," +
            (inputRect.Right - inputRect.Left) + "," + (inputRect.Bottom - inputRect.Top) +
            "],\"work_area\":[" + workArea.Left + "," + workArea.Top + "," +
            (workArea.Right - workArea.Left) + "," + (workArea.Bottom - workArea.Top) +
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
            case "lifecycle-drain":
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
                    Editor.CaretIndex = Math.Max(0, Math.Min(Editor.TextLength, value)); Editor.FocusEditor();
                } else if (command.StartsWith("CreateScenario|")) {
                    var value = command.Substring(15);
                    Scenario = value;
                    Mode = value;
                    Editor.IsReadOnly = false;
                    if (value == "normal") ReplaceEditor("synthetic caret fixture text");
                    else if (value == "empty") { Editor.SetText(""); Editor.CaretIndex = 0; Editor.FocusEditor(); }
                    else if (value == "multiline") ReplaceEditor("line one\r\nline two\r\nline three");
                    else if (value == "context-replaced") ReplaceEditor("replacement TSF context");
                    else if (value == "readonly") { Editor.IsReadOnly = true; Editor.FocusEditor(); }
                    else if (value == "child-two") {
                        Mode = "normal";
                        SwitchEditor(true);
                        ReplaceEditor("second native child fixture text");
                    } else if (value == "child-one") {
                        Mode = "normal";
                        SwitchEditor(false);
                        ReplaceEditor("first native child fixture text");
                    }
                    else if (value == "mixed-dpi") {
                        result = Unsupported("mixed-dpi requires an explicit multi-monitor DPI layout");
                    }
                    else result = Unsupported("scenario requires a real COM fault adapter");
                } else if (command == "FocusEditor") {
                    Editor.FocusEditor();
                } else if (command == "SwitchChild|second") {
                    Scenario = "child-two";
                    SwitchEditor(true);
                } else if (command == "SwitchChild|first") {
                    Scenario = "child-one";
                    SwitchEditor(false);
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
