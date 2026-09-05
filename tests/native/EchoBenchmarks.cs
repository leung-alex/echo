using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading;
using System.Windows.Automation;

// These methods target only the PID/window passed by an isolated native test.
// They use scoped UIA/owned process handles, not clipboard or global key input.
public static class EchoBenchmarks
{
    static void Until(Func<bool> condition, string label, int timeout)
    {
        var clock = Stopwatch.StartNew();
        while (clock.ElapsedMilliseconds < timeout)
        {
            if (condition()) return;
            Thread.Sleep(2);
        }
        throw new TimeoutException(label);
    }

    public static object Search(int pid, string title, int count)
    {
        if (count < 1 || count > 200) throw new ArgumentOutOfRangeException("count");
        var rows = new List<object>();
        for (int index = 0; index < count; index++)
        {
            string query = "echo-perf-text-" + (13 + index * 7).ToString("D4", CultureInfo.InvariantCulture);
            var timer = Stopwatch.StartNew();
            EchoUi.SetValue(pid, title, "Search clipboard history", query);
            Until(() => EchoUi.Elements(pid, title).Any(element =>
            {
                try
                {
                    return element.Current.ControlType != ControlType.Edit &&
                        !element.Current.IsOffscreen && element.Current.Name.IndexOf(query, StringComparison.Ordinal) >= 0;
                }
                catch (ElementNotAvailableException) { return false; }
            }), "Expected search result was not visible: " + query, 5000);
            timer.Stop();
            rows.Add(new { run = index, query = query, elapsed_ms = timer.Elapsed.TotalMilliseconds, status = "PASS" });
        }
        return new { clock = "QPC", frequency = Stopwatch.Frequency, records = rows, note = "Includes UIA input dispatch, actual debounce/query/render and UIA observation; does not measure physical keystrokes." };
    }

    static void Hide(int pid, string title)
    {
        EchoUi.Close(pid, title);
        Until(() => EchoUi.Window(pid, title, true) == IntPtr.Zero && EchoUi.Window(pid, "Echo Favorites", true) == IntPtr.Zero,
            "Owned composition did not hide", 3000);
    }

    public static object Cycles(int pid, string title, string executable, int count)
    {
        if (count < 1 || count > 2000) throw new ArgumentOutOfRangeException("count");
        using (var root = Process.GetProcessById(pid))
        {
            if (!String.Equals(Path.GetFullPath(root.MainModule.FileName), Path.GetFullPath(executable), StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Cycle executable does not match the owned root process");
            var samples = new List<object>();
            var clock = Stopwatch.StartNew();
            const int warmup = 50;
            long basePrivate = 0; int baseHandles = 0; int baseThreads = 0;
            for (int index = 0; index < count + warmup; index++)
            {
                if (root.HasExited) throw new InvalidOperationException("Owned root exited during cycles");
                Hide(pid, title);
                using (var request = Process.Start(new ProcessStartInfo(executable) { UseShellExecute = false, CreateNoWindow = true }))
                {
                    if (!request.WaitForExit(5000))
                    {
                        request.Kill(); request.WaitForExit(5000);
                        throw new TimeoutException("Owned secondary-instance request did not exit");
                    }
                    if (request.ExitCode != 0) throw new InvalidOperationException("Secondary instance failed");
                }
                Until(() => EchoUi.Ready(pid, title) && EchoUi.Window(pid, "Echo Favorites", true) != IntPtr.Zero,
                    "Semantic composition readiness failed during cycle", 5000);
                Hide(pid, title);
                if (index == warmup - 1 || (index >= warmup && ((index + 1 - warmup) % 25 == 0 || index == count + warmup - 1)))
                {
                    Thread.Sleep(20); root.Refresh();
                    if (index == warmup - 1) { basePrivate = root.PrivateMemorySize64; baseHandles = root.HandleCount; baseThreads = root.Threads.Count; }
                    samples.Add(new { cycle = Math.Max(0, index + 1 - warmup), elapsed_ms = clock.Elapsed.TotalMilliseconds,
                        private_bytes = root.PrivateMemorySize64, working_set_bytes = root.WorkingSet64,
                        handles = root.HandleCount, threads = root.Threads.Count });
                }
            }
            Thread.Sleep(2000); root.Refresh();
            long finalPrivate = root.PrivateMemorySize64;
            int finalHandles = root.HandleCount, finalThreads = root.Threads.Count;
            return new { completed_cycles = count, warmup_cycles = warmup, elapsed_ms = clock.Elapsed.TotalMilliseconds,
                initial_private_bytes = basePrivate, final_private_bytes = finalPrivate,
                initial_handles = baseHandles, final_handles = finalHandles,
                initial_threads = baseThreads, final_threads = finalThreads,
                within_growth_budget = finalPrivate <= Math.Max(basePrivate + 8L * 1024 * 1024, (long)(basePrivate * 1.25)) &&
                    finalHandles <= baseHandles + 10 && finalThreads <= baseThreads + 4,
                samples = samples, note = "One finite native-process UI lifecycle stress test, not an eight-hour soak or proof against every leak." };
        }
    }
}
