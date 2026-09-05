using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;

public static class EchoUi
{
    public delegate bool EnumProc(IntPtr hwnd, IntPtr state);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr state);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    public static IntPtr Window(int pid, string title, bool visible)
    {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr hwnd, IntPtr state)
        {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner != (uint)pid || (visible && !IsWindowVisible(hwnd))) return true;
            StringBuilder text = new StringBuilder(512);
            GetWindowText(hwnd, text, text.Capacity);
            if (text.ToString() != title) return true;
            found = hwnd;
            return false;
        }, IntPtr.Zero);
        return found;
    }

    static IntPtr Require(int pid, string title, bool visible)
    {
        IntPtr hwnd = Window(pid, title, visible);
        if (hwnd == IntPtr.Zero) throw new InvalidOperationException("Owned top-level window unavailable: pid=" + pid + " title=" + title);
        return hwnd;
    }

    public static AutomationElement Root(int pid, string title)
    {
        return AutomationElement.FromHandle(Require(pid, title, true));
    }

    // UIA providers may expose a second top-level Slint window below the first
    // root. Only elements whose nearest Window ancestor is the requested HWND
    // are returned, so main-window queries cannot accidentally use Favorites.
    public static List<AutomationElement> Elements(int pid, string title)
    {
        AutomationElement root = Root(pid, title);
        int requestedHandle = root.Current.NativeWindowHandle;
        List<AutomationElement> result = new List<AutomationElement>();
        AutomationElementCollection candidates = root.FindAll(TreeScope.Descendants, Condition.TrueCondition);
        foreach (AutomationElement candidate in candidates)
        {
            try
            {
                AutomationElement ancestor = candidate;
                for (int depth = 0; ancestor != null && depth < 64; depth++)
                {
                    if (ancestor.Current.ControlType == ControlType.Window)
                    {
                        if (ancestor.Current.NativeWindowHandle == requestedHandle) result.Add(candidate);
                        break;
                    }
                    ancestor = TreeWalker.ControlViewWalker.GetParent(ancestor);
                }
            }
            catch (ElementNotAvailableException) { }
        }
        return result;
    }

    public static AutomationElement Find(int pid, string title, string name)
    {
        AutomationElement element = Elements(pid, title).FirstOrDefault(delegate(AutomationElement item)
        {
            try { return item.Current.Name == name; }
            catch (ElementNotAvailableException) { return false; }
        });
        if (element == null) throw new InvalidOperationException("Control unavailable in owned window: " + name);
        return element;
    }

    // Labels and edits can share a name. Never focus the first text label.
    static AutomationElement FindEdit(int pid, string title, string name)
    {
        var element = Elements(pid, title).FirstOrDefault(e =>
            e.Current.Name == name && (e.Current.ControlType == ControlType.Edit || e.Current.ControlType == ControlType.ComboBox));
        if (element == null) throw new InvalidOperationException("Editable control unavailable: " + name);
        return element;
    }

    public static string Dump(int pid, string title)
    {
        StringBuilder output = new StringBuilder();
        foreach (AutomationElement element in Elements(pid, title))
        {
            try
            {
                output.Append(element.Current.ControlType.ProgrammaticName).Append(" | ")
                    .Append(element.Current.Name).Append(" | enabled=").Append(element.Current.IsEnabled)
                    .Append(" | offscreen=").Append(element.Current.IsOffscreen)
                    .Append(" | bounds=").Append(element.Current.BoundingRectangle).AppendLine();
            }
            catch (ElementNotAvailableException) { }
        }
        return output.ToString();
    }

    public static bool Ready(int pid, string title)
    {
        if (Window(pid, title, true) == IntPtr.Zero) return false;
        List<AutomationElement> elements = Elements(pid, title);
        AutomationElement search = elements.FirstOrDefault(delegate(AutomationElement item)
        {
            try { return item.Current.Name == "Search clipboard history" && item.Current.IsEnabled && !item.Current.IsOffscreen; }
            catch (ElementNotAvailableException) { return false; }
        });
        if (search == null) return false;
        return elements.Any(delegate(AutomationElement item)
        {
            try
            {
                string name = item.Current.Name ?? String.Empty;
                return !item.Current.IsOffscreen &&
                    (name.IndexOf("Echo fixture", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("echo-fixture-", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("echo-perf-text-", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("Fixture image", StringComparison.Ordinal) >= 0);
            }
            catch (ElementNotAvailableException) { return false; }
        });
    }

    public static void Focus(int pid, string title)
    {
        IntPtr hwnd = Require(pid, title, true);
        SetForegroundWindow(hwnd);
        Stopwatch wait = Stopwatch.StartNew();
        while (wait.ElapsedMilliseconds < 1000)
        {
            if (GetForegroundWindow() == hwnd) return;
            Thread.Sleep(10);
        }
        throw new InvalidOperationException("Foreground was not granted to the owned Echo window; input cancelled");
    }

    public static void Invoke(int pid, string title, string name)
    {
        // UIA actions are scoped to an owned control; no global input or foreground is required.
        AutomationElement element = Find(pid, title, name);
        if (!element.Current.IsEnabled) throw new InvalidOperationException("Control disabled: " + name);
        object pattern;
        if (!element.TryGetCurrentPattern(InvokePattern.Pattern, out pattern)) throw new InvalidOperationException("No InvokePattern: " + name);
        ((InvokePattern)pattern).Invoke();
    }

    public static void Select(int pid, string title, string name)
    {
        // UIA actions are scoped to an owned control; no global input or foreground is required.
        AutomationElement element = Find(pid, title, name);
        object pattern;
        if (element.TryGetCurrentPattern(SelectionItemPattern.Pattern, out pattern))
        {
            ((SelectionItemPattern)pattern).Select();
            return;
        }
        if (element.TryGetCurrentPattern(InvokePattern.Pattern, out pattern))
        {
            ((InvokePattern)pattern).Invoke();
            return;
        }
        throw new InvalidOperationException("No selection/default action: " + name);
    }

    public static void SetValue(int pid, string title, string name, string value)
    {
        // UIA actions are scoped to an owned control; no global input or foreground is required.
        AutomationElement element = FindEdit(pid, title, name);
        object pattern;
        if (!element.TryGetCurrentPattern(ValuePattern.Pattern, out pattern)) throw new InvalidOperationException("No ValuePattern: " + name);
        ((ValuePattern)pattern).SetValue(value);
        Stopwatch wait = Stopwatch.StartNew();
        string actual = String.Empty;
        while (wait.ElapsedMilliseconds < 5000)
        {
            element = FindEdit(pid, title, name);
            if (element.TryGetCurrentPattern(ValuePattern.Pattern, out pattern))
            {
                actual = ((ValuePattern)pattern).Current.Value;
                if (actual == value) return;
            }
            if (element.TryGetCurrentPattern(TextPattern.Pattern, out pattern))
            {
                actual = ((TextPattern)pattern).DocumentRange.GetText(-1).TrimEnd('\r', '\n');
                if (actual == value) return;
            }
            Thread.Sleep(15);
        }
        throw new TimeoutException("UIA value did not converge: " + name + " expected=" + value + " actual=" + actual);
    }

    public static string ReadText(int pid, string title, string name)
    {
        AutomationElement element = FindEdit(pid, title, name);
        object pattern;
        if (element.TryGetCurrentPattern(ValuePattern.Pattern, out pattern)) return ((ValuePattern)pattern).Current.Value;
        if (element.TryGetCurrentPattern(TextPattern.Pattern, out pattern)) return ((TextPattern)pattern).DocumentRange.GetText(-1).TrimEnd('\r', '\n');
        return element.Current.Name;
    }

    public static void SetTheme(int pid, string title, string value)
    {
        if(value!="system" && value!="light" && value!="dark") throw new ArgumentException("Invalid theme");
        var element = Elements(pid,title).FirstOrDefault(e=>e.Current.ControlType==ControlType.ComboBox && e.Current.Name=="Theme");
        if(element==null)throw new InvalidOperationException("Theme combobox unavailable");
        Focus(pid,title); element.SetFocus();
        Key(pid,title,0x24,false,false);
        int steps=value=="dark"?2:value=="light"?1:0;
        for(int i=0;i<steps;i++) Key(pid,title,0x28,false,false);
        Key(pid,title,0x0D,false,false);
    }

    public static void Key(int pid, string title, byte code, bool control, bool shift)
    {
        Focus(pid, title);
        IntPtr hwnd = Require(pid, title, true);
        if (GetForegroundWindow() != hwnd) throw new InvalidOperationException("Foreground changed before keyboard input; input cancelled");
        if (control) keybd_event(0x11, 0, 0, UIntPtr.Zero);
        if (shift) keybd_event(0x10, 0, 0, UIntPtr.Zero);
        try
        {
            if (GetForegroundWindow() != hwnd) throw new InvalidOperationException("Foreground changed during keyboard input; input cancelled");
            keybd_event(code, 0, 0, UIntPtr.Zero);
            keybd_event(code, 0, 2, UIntPtr.Zero);
        }
        finally
        {
            if (shift) keybd_event(0x10, 0, 2, UIntPtr.Zero);
            if (control) keybd_event(0x11, 0, 2, UIntPtr.Zero);
        }
    }

    public static void Hover(int pid, string title, string name)
    {
        Focus(pid, title);
        AutomationElement element = Find(pid, title, name);
        System.Windows.Rect bounds = element.Current.BoundingRectangle;
        Rect window;
        IntPtr hwnd = Require(pid, title, true);
        GetWindowRect(hwnd, out window);
        int x = (int)(bounds.Left + bounds.Width / 2);
        int y = (int)(bounds.Top + bounds.Height / 2);
        if (x < window.Left || x >= window.Right || y < window.Top || y >= window.Bottom) throw new InvalidOperationException("Control lies outside the owned window: " + name);
        if (GetForegroundWindow() != hwnd) throw new InvalidOperationException("Foreground changed before pointer movement; input cancelled");
        SetCursorPos(x, y);
    }

    public static void Close(int pid, string title)
    {
        PostMessage(Require(pid, title, true), 0x0010, IntPtr.Zero, IntPtr.Zero);
    }

    public static void Capture(int pid, string title, string path)
    {
        if (System.IO.File.Exists(path)) throw new InvalidOperationException("Screenshot path already exists: " + path);
        Focus(pid, title);
        IntPtr hwnd = Require(pid, title, true);
        IntPtr old = SetThreadDpiAwarenessContext(new IntPtr(-4));
        try
        {
            Rect rect;
            GetWindowRect(hwnd, out rect);
            int width = rect.Right - rect.Left;
            int height = rect.Bottom - rect.Top;
            if (width < 1 || height < 1) throw new InvalidOperationException("Invalid owned-window geometry");
            using (Bitmap bitmap = new Bitmap(width, height, PixelFormat.Format32bppArgb))
            using (Graphics graphics = Graphics.FromImage(bitmap))
            {
                graphics.CopyFromScreen(rect.Left, rect.Top, 0, 0, bitmap.Size, CopyPixelOperation.SourceCopy);
                bitmap.Save(path, ImageFormat.Png);
            }
        }
        finally { SetThreadDpiAwarenessContext(old); }
    }

    public static void Resize(int pid, string title, int width, int height)
    {
        SetWindowPos(Require(pid, title, true), IntPtr.Zero, 0, 0, width, height, 0x0002 | 0x0004 | 0x0010);
    }
}
