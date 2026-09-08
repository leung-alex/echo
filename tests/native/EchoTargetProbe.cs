// Read-only, content-free capability probe for an explicitly selected GUI HWND.
// Never reads names, values, text ranges or clipboard contents; never sends input.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows.Automation;
using System.Web.Script.Serialization;

public static class EchoTargetProbe {
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [STAThread] public static int Main(string[] args) {
        try {
            if (args.Length != 2) throw new ArgumentException("Expected GUI PID and root HWND");
            int pid = Int32.Parse(args[0]);
            var hwnd = new IntPtr(Int64.Parse(args[1]));
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != pid || hwnd == IntPtr.Zero || GetAncestor(hwnd, 2) != hwnd)
                throw new InvalidOperationException("The explicit root HWND does not belong to this GUI PID");
            var process = Process.GetProcessById(pid);
            var root = AutomationElement.FromHandle(hwnd);
            var clock = Stopwatch.StartNew();
            var items = new List<object>();
            var queue = new Queue<AutomationElement>(); queue.Enqueue(root);
            int visited = 0;
            while (queue.Count > 0 && visited < 512 && clock.ElapsedMilliseconds < 3000) {
                var element = queue.Dequeue(); visited++;
                try {
                    var c = element.Current;
                    if (c.ControlType == ControlType.Edit || c.ControlType == ControlType.Document || c.HasKeyboardFocus) {
                        var patterns = new List<string>();
                        foreach (var p in element.GetSupportedPatterns()) patterns.Add(p.ProgrammaticName);
                        items.Add(new { runtime_id = element.GetRuntimeId(), pid = c.ProcessId,
                            role = c.ControlType.ProgrammaticName, focused = c.HasKeyboardFocus,
                            offscreen = c.IsOffscreen, enabled = c.IsEnabled, password = c.IsPassword,
                            hwnd = c.NativeWindowHandle, patterns = patterns });
                    }
                    var child = TreeWalker.RawViewWalker.GetFirstChild(element);
                    while (child != null && queue.Count < 512) {
                        queue.Enqueue(child); child = TreeWalker.RawViewWalker.GetNextSibling(child);
                    }
                } catch (ElementNotAvailableException) { }
            }
            Console.WriteLine(new JavaScriptSerializer().Serialize(new { status = "OBSERVED",
                pid = pid, hwnd = hwnd.ToInt64(), executable = process.MainModule.FileName,
                process_created = process.StartTime.ToUniversalTime().ToString("o"),
                visited = visited, elapsed_ms = clock.ElapsedMilliseconds,
                truncated = queue.Count > 0, elements = items,
                limitation = "Advertised patterns do not prove exact selection, replacement or IME support." }));
            return 0;
        } catch (Exception e) {
            Console.WriteLine(new JavaScriptSerializer().Serialize(new { status = "FAIL", error = e.Message }));
            return 1;
        }
    }
}
