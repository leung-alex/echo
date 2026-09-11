using System;
using System.Web.Script.Serialization;
using System.Windows.Automation;

public static class EchoSmokeDriver
{
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer { MaxJsonLength = 8 * 1024 * 1024 };

    public static int Main(string[] args)
    {
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        try
        {
            if (args.Length < 3) throw new ArgumentException("operation, owned PID, and exact top-level window title are required");
            int pid = Int32.Parse(args[1]);
            string title = args[2];
            object value = null;
            switch (args[0])
            {
                case "window-count": value=EchoUi.WindowCount(pid); break;
                case "ready": value = EchoUi.Ready(pid, title); break;
                case "dump": value = EchoUi.Dump(pid, title); break;
                case "close": EchoUi.Close(pid, title); break;
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
