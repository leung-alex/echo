// Owned synthetic WPF editor used by the caret geometry acceptance runner.
// It contains only synthetic text and exposes a nonce-free local named pipe
// whose name is written to fixture.ready.json. Echo is the only client used by
// the runner; no arbitrary HWND or process can be commanded by this fixture.
using System;
using System.Diagnostics;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Threading;

public static class CaretTsfFixture {
    static readonly object Gate = new object();
    static string Root;
    static TextBox Editor;
    static Window Window;
    static string Mode = "normal";
    static bool Stop;
    static int ScenarioCaret;

    [STAThread]
    public static int Main(string[] args) {
        if (args.Length != 1 || Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE") != "1") return 2;
        Root = Path.GetFullPath(args[0]);
        Directory.CreateDirectory(Root);
        var app = new Application();
        Window = new Window { Width = 720, Height = 360, Left = 620, Top = 120,
            WindowStartupLocation = WindowStartupLocation.Manual, Title = "Echo owned caret fixture" };
        var panel = new StackPanel { Margin = new Thickness(24) };
        panel.Children.Add(new TextBlock { Text = "Owned synthetic TSF editor", FontSize = 18, Margin = new Thickness(0,0,0,12) });
        Editor = new TextBox { Width = 620, Height = 180, FontSize = 18,
            Text = "synthetic caret fixture text", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap };
        panel.Children.Add(Editor);
        Window.Content = panel;
        Window.ContentRendered += delegate {
            Editor.Focus(); Editor.CaretIndex = Editor.Text.Length;
            WriteReady(); WriteOracle();
        };
        var timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(20) };
        timer.Tick += delegate { if (Stop) { timer.Stop(); Window.Close(); } else WriteOracle(); };
        timer.Start();
        var pipeThread = new Thread(PipeLoop) { IsBackground = true, Name = "caret-fixture-pipe" };
        pipeThread.Start();
        app.Run(Window);
        Stop = true;
        return 0;
    }

    static void WriteReady() {
        var handle = new WindowInteropHelper(Window).Handle;
        File.WriteAllText(Path.Combine(Root, "fixture.ready.json"),
            "{\"pid\":" + Process.GetCurrentProcess().Id + ",\"hwnd\":" + handle.ToInt64() + ",\"pipe\":\"EchoCaretFixture-" + Process.GetCurrentProcess().Id + "\"}");
    }

    static void WriteOracle() {
        if (Editor == null || Window == null) return;
        try {
            var index = Math.Max(0, Math.Min(Editor.Text.Length, Editor.CaretIndex));
            var r = Editor.GetRectFromCharacterIndex(Math.Max(0, index - 1), true);
            var p = Editor.PointToScreen(new Point(r.Right, r.Top));
            var scale = PresentationSource.FromVisual(Editor).CompositionTarget.TransformToDevice;
            var height = Math.Max(1.0, r.Height * scale.M22);
            var line = "{\"pid\":" + Process.GetCurrentProcess().Id + ",\"caret\":[" +
                ((int)Math.Round(p.X)) + "," + ((int)Math.Round(p.Y)) + "," +
                ((int)Math.Round(p.X) + 1) + "," + ((int)Math.Round(p.Y + height)) +
                "],\"mode\":\"" + Mode + "\",\"sequence\":" + (++ScenarioCaret) + "}";
            var tmp = Path.Combine(Root, "fixture.oracle.tmp");
            File.WriteAllText(tmp, line, Encoding.UTF8);
            var path = Path.Combine(Root, "fixture.oracle.json");
            if (File.Exists(path)) File.Delete(path);
            File.Move(tmp, path);
        } catch { }
    }

    static void PipeLoop() {
        var name = "EchoCaretFixture-" + Process.GetCurrentProcess().Id;
        while (!Stop) {
            try {
                using (var pipe = new NamedPipeServerStream(name, PipeDirection.InOut, 1, PipeTransmissionMode.Byte, PipeOptions.Asynchronous)) {
                    pipe.WaitForConnection();
                    using (var reader = new StreamReader(pipe, Encoding.UTF8, false, 1024, true))
                    using (var writer = new StreamWriter(pipe, Encoding.UTF8, 1024, true) { AutoFlush = true }) {
                        var command = reader.ReadLine() ?? "";
                        var result = Dispatch(command);
                        writer.WriteLine(result);
                    }
                }
            } catch { Thread.Sleep(20); }
        }
    }

    static string Dispatch(string command) {
        try {
            Window.Dispatcher.Invoke(DispatcherPriority.Send, new Action(delegate {
                if (command.StartsWith("MoveCaret|")) {
                    int value; if (int.TryParse(command.Substring(10), out value)) Editor.CaretIndex = Math.Max(0, Math.Min(Editor.Text.Length, value));
                } else if (command.StartsWith("CreateScenario|")) {
                    Mode = command.Substring(15); Editor.IsReadOnly = false; Editor.Focus();
                } else if (command.StartsWith("SetSyntheticModeForFixture|")) {
                    Mode = command.Substring(27);
                } else if (command == "FocusEditor") {
                    Editor.Focus();
                } else if (command == "SetFault|readonly") {
                    Editor.IsReadOnly = true;
                } else if (command == "ShutdownOwnedFixture") {
                    Stop = true;
                }
                WriteOracle();
            }));
            return "{\"status\":\"PASS\"}";
        } catch { return "{\"status\":\"FAIL\"}"; }
    }
}
