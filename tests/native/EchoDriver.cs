using System;
using System.Web.Script.Serialization;
using System.Windows.Automation;

public static class EchoDriver
{
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer { MaxJsonLength = 8 * 1024 * 1024 };

    public static int Main(string[] args)
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        try
        {
            if (Environment.GetEnvironmentVariable("ECHO_WINDOWS_ACCEPTANCE") != "1") throw new InvalidOperationException("Native acceptance requires ECHO_WINDOWS_ACCEPTANCE=1");
            if (args.Length < 3) throw new ArgumentException("operation, owned PID, and exact top-level window title are required");
            int pid = Int32.Parse(args[1]);
            string title = args[2];
            object value = null;
            switch (args[0])
            {
                case "benchmark-search": Require(args,4); value=EchoBenchmarks.Search(pid,title,Int32.Parse(args[3])); break;
                case "benchmark-cycles": Require(args,5); value=EchoBenchmarks.Cycles(pid,title,args[3],Int32.Parse(args[4])); break;
                case "window-count": value=EchoUi.WindowCount(pid); break;
                case "combo": Require(args,5); EchoUi.Combo(pid,title,args[3],Int32.Parse(args[4])); break;
                case "toggle": Require(args,5); EchoUi.Toggle(pid,title,args[3],Boolean.Parse(args[4])); break;
                case "group-invoke": Require(args,5); EchoUi.InvokeInGroup(pid,title,args[3],args[4]); break;
                case "composition": Require(args,4); value=EchoComposition.Check(pid,title,args[3]); break;
                case "scroll": Require(args,4); value=EchoTestBridge.Call(pid,"scroll",Int32.Parse(args[3]),false,args.Length>4 && args[4]=="up",""); break;
                case "metrics": value = EchoTestBridge.Call(pid,"metrics",0,false,false,""); break;
                case "reset-metrics": value = EchoTestBridge.Call(pid,"reset_metrics",0,false,false,""); break;
                case "ready": value = EchoUi.Ready(pid, title); break;
                case "dump": value = EchoUi.Dump(pid, title); break;
                case "capture": Require(args, 4); EchoUi.Capture(pid, title, args[3]); break;
                case "invoke": Require(args, 4); EchoUi.Invoke(pid, title, args[3]); break;
                case "select": Require(args, 4); EchoUi.Select(pid, title, args[3]); break;
                case "value": Require(args, 5); EchoUi.SetValue(pid, title, args[3], args[4]); break;
                case "read": Require(args, 4); value = EchoUi.ReadText(pid, title, args[3]); break;
                case "theme": Require(args, 4); EchoUi.SetTheme(pid, title, args[3]); break;
                case "key": Require(args, 4); EchoUi.Key(pid, title, Byte.Parse(args[3]), args.Length > 4 && args[4] == "ctrl", args.Length > 5 && args[5] == "shift"); break;
                case "close": EchoUi.Close(pid, title); break;
                case "focus": EchoUi.Focus(pid, title); break;
                case "hover": Require(args, 4); EchoUi.Hover(pid, title, args[3]); break;
                case "resize": Require(args, 5); EchoUi.Resize(pid, title, Int32.Parse(args[3]), Int32.Parse(args[4])); break;
                case "exists": value = EchoUi.Window(pid, title, true) != IntPtr.Zero; break;
                default: throw new ArgumentException("Unknown operation: " + args[0]);
            }
            Console.WriteLine(Json.Serialize(new { status = "PASS", operation = args[0], value = value }));
            return 0;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(Json.Serialize(new { status = "FAIL", error = error.ToString() }));
            return 1;
        }
    }

    static void Require(string[] args, int count)
    {
        if (args.Length < count) throw new ArgumentException("Insufficient operation arguments");
    }
}
