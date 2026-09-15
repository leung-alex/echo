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
    static IntPtr explicitWindow;
    static int explicitPid;
    static string explicitTitle;
    static string[] explicitTitles;
    // One command process, one explicitly registered application window. This
    // prevents two same-title Codex windows from sharing an input lookup.
    public static void BindWindow(int pid, string title, long hwnd)
    {
        explicitPid = pid; explicitTitle = title; explicitWindow = new IntPtr(hwnd);
        explicitTitles = new[]{title};
    }
    public static void BindWindowTitles(int pid, string title, long hwnd, string[] titles)
    {
        if(titles == null || titles.Length == 0 || !titles.Contains(title) || titles.Any(String.IsNullOrEmpty))throw new ArgumentException("Explicit window titles must include the registered title");
        BindWindow(pid,title,hwnd);
        explicitTitles = titles;
    }
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
        if (explicitWindow != IntPtr.Zero && pid == explicitPid && title == explicitTitle)
        {
            uint owner; GetWindowThreadProcessId(explicitWindow, out owner);
            var label = new StringBuilder(512); GetWindowText(explicitWindow, label, label.Capacity);
            return owner == (uint)pid && explicitTitles.Contains(label.ToString()) && (!visible || IsWindowVisible(explicitWindow))
                ? explicitWindow : IntPtr.Zero;
        }
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
            { try { return e.Current.Name == name && (e.Current.ControlType == ControlType.Edit || e.Current.ControlType == ControlType.ComboBox); } catch(ElementNotAvailableException) { return false; } });
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
        AutomationElement space = elements.FirstOrDefault(delegate(AutomationElement item)
        {
            try { return item.Current.Name == "History space" && item.Current.IsEnabled && !item.Current.IsOffscreen; }
            catch (ElementNotAvailableException) { return false; }
        });
        if (space == null) return false;
        return elements.Any(delegate(AutomationElement item)
        {
            try
            {
                string name = item.Current.Name ?? String.Empty;
                return !item.Current.IsOffscreen &&
                    (name.IndexOf("Echo fixture", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("echo-fixture-", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("echo-perf-text-", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("Fixture image", StringComparison.Ordinal) >= 0 ||
                     name.IndexOf("Your clipboard history appears here", StringComparison.Ordinal) >= 0);
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

    public static void InvokeInGroup(int pid,string title,string group,string name)
    {
        var parent=Find(pid,title,group);
        var control=parent.FindFirst(TreeScope.Descendants,new PropertyCondition(AutomationElement.NameProperty,name));
        if(control==null || !control.Current.IsEnabled)throw new InvalidOperationException("Owned group control unavailable: "+group+" / "+name);
        object pattern;if(!control.TryGetCurrentPattern(InvokePattern.Pattern,out pattern))throw new InvalidOperationException("Missing InvokePattern");
        ((InvokePattern)pattern).Invoke();
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
        Combo(pid,title,"Theme",value=="dark"?2:value=="light"?1:0);
    }

    public static void Key(int pid, string title, byte code, bool control, bool shift)
    {
        // The diagnostic bridge has no F10 entry. Exercise the real owned-window
        // context-menu shortcut through the foreground-checked OS path below.
        // All other diagnostic keys retain the restricted framework bridge.
        bool contextMenu = code == 121 && shift && !control;
        if (EchoTestBridge.Enabled && !contextMenu) { EchoTestBridge.Call(pid, "key", code, control, shift, ""); return; }
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
        if (!EchoTestBridge.Enabled)
            throw new InvalidOperationException("Captures require the isolated native-test renderer bridge; desktop screenshots are disabled.");
        EchoTestBridge.Capture(pid,path);
    }

    public static void Resize(int pid, string title, int width, int height)
    {
        SetWindowPos(Require(pid, title, true), IntPtr.Zero, 0, 0, width, height, 0x0002 | 0x0004 | 0x0010);
    }
    public static int WindowCount(int pid)
    {
        int count=0;
        EnumWindows(delegate(IntPtr hwnd,IntPtr state) {
            uint owner;GetWindowThreadProcessId(hwnd,out owner);
            if(owner==(uint)pid && IsWindowVisible(hwnd)) {
                var title=new StringBuilder(512);GetWindowText(hwnd,title,title.Capacity);
                if(title.ToString().StartsWith("Echo",StringComparison.Ordinal)) count++;
            }
            return true;
        },IntPtr.Zero);
        return count;
    }
    public static void Combo(int pid,string title,string name,int index)
    {
        if(index<0 || index>12) throw new ArgumentException("Invalid combo index");
        var element=FindEdit(pid,title,name);
        if(element.Current.ControlType!=ControlType.ComboBox) throw new InvalidOperationException("Not a combobox");
        if (!EchoTestBridge.Enabled) Focus(pid,title);
        element.SetFocus();
        // Slint 1.17 ComboBox supports arrows, not Home. Enter opens a popup;
        // selection changes on arrows. Reset within the bounded index contract.
        for(int i=0;i<12;i++) Key(pid,title,0x26,false,false);
        for(int i=0;i<index;i++) Key(pid,title,0x28,false,false);
    }
    public static void Expand(int pid,string title,string name)
    {
        var element=FindEdit(pid,title,name);object pattern;
        if(!element.Current.IsEnabled || !element.TryGetCurrentPattern(ExpandCollapsePattern.Pattern,out pattern))
            throw new InvalidOperationException("No enabled ExpandCollapsePattern: "+name);
        ((ExpandCollapsePattern)pattern).Expand();
    }
    public static object PopupItems(int pid,string title)
    {
        return Root(pid,title).FindAll(TreeScope.Descendants,new PropertyCondition(AutomationElement.ControlTypeProperty,ControlType.ListItem))
            .Cast<AutomationElement>().Select(e => {var b=e.Current.BoundingRectangle;return new {name=e.Current.Name,bounds=new[]{b.Left,b.Top,b.Width,b.Height}};}).ToArray();
    }
    public static void SelectPopupItem(int pid,string title,string name)
    {
        // Popup rows have their own Window ancestor in the accessibility tree.
        // Search only below the registered owner, without the normal main-view filter.
        var element=Root(pid,title).FindAll(TreeScope.Descendants,new PropertyCondition(AutomationElement.ControlTypeProperty,ControlType.ListItem))
            .Cast<AutomationElement>().FirstOrDefault(e => e.Current.Name==name && e.Current.IsEnabled && !e.Current.IsOffscreen);
        if(element==null)throw new InvalidOperationException("Owned popup row unavailable: "+name);
        object pattern;
        if(element.TryGetCurrentPattern(InvokePattern.Pattern,out pattern)){((InvokePattern)pattern).Invoke();return;}
        if(element.TryGetCurrentPattern(SelectionItemPattern.Pattern,out pattern)){((SelectionItemPattern)pattern).Select();return;}
        throw new InvalidOperationException("No popup selection/default action: "+name);
    }
    public static void Toggle(int pid,string title,string name,bool enabled)
    {
        var element=Find(pid,title,name);object pattern;
        if(!element.TryGetCurrentPattern(TogglePattern.Pattern,out pattern)) throw new InvalidOperationException("No TogglePattern: "+name);
        var toggle=(TogglePattern)pattern;
        if((toggle.Current.ToggleState==ToggleState.On)!=enabled) toggle.Toggle();
    }

}

// Explicit test builds only. Never sends OS keyboard events or captures the desktop.
public static class EchoTestBridge {
    public static bool Enabled { get { return !String.IsNullOrEmpty(Environment.GetEnvironmentVariable("ECHO_NATIVE_TEST_ROOT")); } }
    public static void Capture(int pid,string path) {
        var root=System.IO.Path.GetFullPath(Environment.GetEnvironmentVariable("ECHO_NATIVE_TEST_ROOT"));
        path=System.IO.Path.GetFullPath(path);
        if(!String.Equals(System.IO.Path.GetDirectoryName(path),root,StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("Capture must be inside the isolated evidence directory");
        Call(pid,"capture",0,false,false,System.IO.Path.GetFileName(path));
    }
    public static object Call(int pid,string verb,int key,bool ctrl,bool shift,string file) {
        if(Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE")!="1") throw new InvalidOperationException("Acceptance not authorized");
        var root=System.IO.Path.GetFullPath(Environment.GetEnvironmentVariable("ECHO_NATIVE_TEST_ROOT"));
        var dir=System.IO.Path.Combine(root,"native-control");
        var json=new System.Web.Script.Serialization.JavaScriptSerializer {MaxJsonLength=8*1024*1024};
        var id=Guid.NewGuid().ToString("N");
        var request=System.IO.Path.Combine(dir,"request.json");
        var temporary=System.IO.Path.Combine(dir,"request.pending");
        System.IO.File.WriteAllText(temporary,json.Serialize(new {id=id,pid=pid,verb=verb,key=key,ctrl=ctrl,shift=shift,file=file}),new UTF8Encoding(false));
        if(System.IO.File.Exists(request))System.IO.File.Delete(request);
        System.IO.File.Move(temporary,request);
        var response=System.IO.Path.Combine(dir,"response.json");
        var watch=Stopwatch.StartNew();
        while(watch.ElapsedMilliseconds<15000) {
            try {
                if(System.IO.File.Exists(response)) {
                    var result=json.Deserialize<Dictionary<string,object>>(System.IO.File.ReadAllText(response));
                    if((string)result["id"]==id) {
                        if((string)result["status"]!="PASS")throw new InvalidOperationException("Owned-window test failed: "+result["error"]);
                        return result.ContainsKey("value")?result["value"]:null;
                    }
                }
            } catch(System.IO.IOException) { }
            Thread.Sleep(10);
        }
        throw new TimeoutException("Native test bridge did not respond");
    }
}
