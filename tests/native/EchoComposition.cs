using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

// Samples only pixels currently owned by the launched Echo and this synthetic underlay.
// No desktop screenshot is taken or written. Occluded candidates are not sampled.
// Distributed owned-pixel coverage is mandatory; visibility changes during a read abort.
public static class EchoComposition
{
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref Point p);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x,int y,int w,int height,uint flags);
    [DllImport("user32.dll",EntryPoint="GetWindowLongPtrW")] static extern IntPtr GetWindowLongPtr(IntPtr h,int index);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    static string Describe(IntPtr h){Rect r;GetWindowRect(h,out r);return r.Left+","+r.Top+","+r.Right+","+r.Bottom+" visible="+IsWindowVisible(h)+" ex="+GetWindowLongPtr(h,-20).ToInt64().ToString("X");}
    [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("user32.dll")] static extern int GetWindowRgn(IntPtr h, IntPtr region);
    [DllImport("gdi32.dll")] static extern IntPtr CreateRectRgn(int l,int t,int r,int b);
    [DllImport("gdi32.dll")] static extern bool PtInRegion(IntPtr r,int x,int y);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr r);
    [DllImport("gdi32.dll")] static extern uint GetPixel(IntPtr dc,int x,int y);
    [DllImport("dwmapi.dll")] static extern int DwmFlush();
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    static readonly Color A=Color.FromArgb(48,80,112), B=Color.FromArgb(170,204,170);
    sealed class Underlay : Form
    {
        public Underlay() {FormBorderStyle=FormBorderStyle.None;ShowInTaskbar=false;DoubleBuffered=true;StartPosition=FormStartPosition.Manual;}
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override void OnPaint(PaintEventArgs e)
        {
            using(var a=new SolidBrush(A)) using(var b=new SolidBrush(B))
                for(int y=0;y<Height;y+=32) for(int x=0;x<Width;x+=32)
                    e.Graphics.FillRectangle(((x/32+y/32)%2)==0?a:b,x,y,32,32);
        }
    }
    public static object Check(int pid,string title,string snapshot)
    {
        object result=null;Exception failure=null;
        var thread=new Thread(delegate(){try{result=CheckCore(pid,title,snapshot);}catch(Exception e){failure=e;}});
        thread.SetApartmentState(ApartmentState.STA);thread.IsBackground=true;thread.Start();
        if(!thread.Join(12000))throw new TimeoutException("Owned composition proof timed out.");
        if(failure!=null)throw new InvalidOperationException("Owned composition proof failed",failure);
        return result;
    }
    static object CheckCore(int pid,string title,string snapshot)
    {
        if(!EchoTestBridge.Enabled) throw new InvalidOperationException("Composition proof requires the isolated native test bridge.");
        string root=Path.GetFullPath(Environment.GetEnvironmentVariable("ECHO_NATIVE_TEST_ROOT"));
        snapshot=Path.GetFullPath(snapshot);
        if(!String.Equals(Path.GetDirectoryName(snapshot),root,StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Only an owned evidence PNG is accepted.");
        IntPtr echo=EchoUi.Window(pid,title,true);
        if(echo==IntPtr.Zero)throw new InvalidOperationException("Owned Echo window is unavailable.");
        IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));
        IntPtr region=CreateRectRgn(0,0,0,0), dc=IntPtr.Zero;
        bool wasTopmost=(GetWindowLongPtr(echo,-20).ToInt64() & 8)!=0;
        try
        {
            using(var image=new Bitmap(snapshot)) using(var underlay=new Underlay())
            {
                Point origin=new Point();Rect window;
                if(!ClientToScreen(echo,ref origin)||!GetWindowRect(echo,out window))throw new InvalidOperationException("Owned geometry unavailable.");
                if(region==IntPtr.Zero || GetWindowRgn(echo,region)==0)throw new InvalidOperationException("Card hit-test region was not installed.");
                underlay.Bounds=new Rectangle(origin.X,origin.Y,image.Width,image.Height);
                underlay.TopMost=true;
                underlay.Show();Application.DoEvents();
                // Own only these two surfaces, without stealing keyboard focus or
                // copying unrelated desktop pixels. Restore the app's band below.
                if(!SetWindowPos(echo,new IntPtr(-1),0,0,0,0,0x0013))
                    throw new InvalidOperationException("Could not stage the owned Echo window.");
                if(!SetWindowPos(underlay.Handle,new IntPtr(-1),origin.X,origin.Y,image.Width,image.Height,0x0010))
                    throw new InvalidOperationException("Could not stage the owned synthetic underlay.");
                if(!SetWindowPos(echo,new IntPtr(-1),0,0,0,0,0x0013))throw new InvalidOperationException("Cannot raise the owned card above its underlay.");
                Thread.Sleep(80);Application.DoEvents();DwmFlush();
                Rect underlayBounds;GetWindowRect(underlay.Handle,out underlayBounds);
                dc=GetDC(IntPtr.Zero);if(dc==IntPtr.Zero)throw new InvalidOperationException("Display sampling unavailable.");
                bool expectAlpha=Environment.GetEnvironmentVariable("ECHO_RENDERER")!="software";
                int count=0,inside=0,largest=0,occluded=0;
                var innerSamples=new List<Point>();var outerSamples=new List<Point>();
                for(int y=11;y<image.Height-10;y+=13)for(int x=13;x<image.Width-12;x+=13)
                {
                    bool included=PtInRegion(region,origin.X-window.Left+x,origin.Y-window.Top+y);
                    if(expectAlpha ? image.GetPixel(x,y).A!=0 : included)continue;
                    (included?innerSamples:outerSamples).Add(new Point{X=x,Y=y});
                }
                var samples=new List<Point>();
                foreach(var bucket in new[]{innerSamples,outerSamples}) {
                    int limit=bucket==innerSamples?64:160;
                    int n=Math.Min(limit,bucket.Count);
                    for(int k=0;k<n;k++)samples.Add(bucket[k*bucket.Count/n]);
                }
                // Screen GetPixel can synchronize with DWM. Sample a bounded,
                // distributed set instead of thousands of synchronous calls.
                foreach(var local in samples) {
                    int x=local.X,y=local.Y;
                    bool inRegion=PtInRegion(region,origin.X-window.Left+x,origin.Y-window.Top+y);
                    Point p=new Point{X=origin.X+x,Y=origin.Y+y};
                    IntPtr before=GetAncestor(WindowFromPoint(p),2);
                    if(before!=echo && before!=underlay.Handle){occluded++;continue;}
                    uint pixel=GetPixel(dc,p.X,p.Y);
                    IntPtr after=GetAncestor(WindowFromPoint(p),2);
                    if(after!=echo && after!=underlay.Handle)throw new InvalidOperationException("Visibility changed at "+p.X+","+p.Y+" echo=["+Describe(echo)+"] underlay=["+Describe(underlay.Handle)+"]; no pixel evidence retained.");
                    Color expected=((x/32+y/32)%2)==0?A:B;
                    int difference=Math.Max(Math.Abs((int)(pixel&255)-expected.R),Math.Max(Math.Abs((int)((pixel>>8)&255)-expected.G),Math.Abs((int)((pixel>>16)&255)-expected.B)));
                    largest=Math.Max(largest,difference);count++;
                    if(inRegion)inside++;
                }
                int opaqueCount=0,opaqueDifference=0;
                for(int y=31;y<image.Height-32 && opaqueCount<16;y+=41)
                for(int x=29;x<image.Width-30 && opaqueCount<16;x+=47) {
                    Color expected=image.GetPixel(x,y);
                    if(expected.A!=255 || expected.ToArgb()!=image.GetPixel(x+2,y+2).ToArgb()
                        || expected.ToArgb()!=image.GetPixel(x-2,y-2).ToArgb()
                        || !PtInRegion(region,origin.X-window.Left+x,origin.Y-window.Top+y))continue;
                    Point point=new Point{X=origin.X+x,Y=origin.Y+y};
                    if(GetAncestor(WindowFromPoint(point),2)!=echo)continue;
                    uint pixel=GetPixel(dc,point.X,point.Y);
                    if(GetAncestor(WindowFromPoint(point),2)!=echo)
                        throw new InvalidOperationException("Owned card visibility changed while validating opaque content.");
                    int difference=Math.Max(Math.Abs((int)(pixel&255)-expected.R),Math.Max(Math.Abs((int)((pixel>>8)&255)-expected.G),Math.Abs((int)((pixel>>16)&255)-expected.B)));
                    opaqueDifference=Math.Max(opaqueDifference,difference);opaqueCount++;
                }
                if(opaqueCount<12 || opaqueDifference>8)
                    throw new InvalidOperationException("Opaque card is not correctly visible above its synthetic underlay: samples="+opaqueCount+" difference="+opaqueDifference);
                if(count<100 || (expectAlpha && inside<20) || largest>8 || occluded>samples.Count/4)
                    throw new InvalidOperationException("Transparent composition mismatch: samples="+count+" inside-region="+inside+" max-difference="+largest+" occluded="+occluded+" underlay="+underlayBounds.Left+","+underlayBounds.Top+","+underlayBounds.Right+","+underlayBounds.Bottom+" origin="+origin.X+","+origin.Y);
                return new { transparent_samples=count,inside_native_region=inside,maximum_channel_difference=largest,skipped_occluded_points=occluded,opaque_card_samples=opaqueCount,opaque_maximum_difference=opaqueDifference,
                    desktop_screenshot=false,source="Owned Echo over an owned synthetic checkerboard" };
            }
        }
        finally {
            if(!wasTopmost)SetWindowPos(echo,new IntPtr(-2),0,0,0,0,0x0013);
            if(dc!=IntPtr.Zero)ReleaseDC(IntPtr.Zero,dc);
            if(region!=IntPtr.Zero)DeleteObject(region);
            SetThreadDpiAwarenessContext(old);
        }
    }
}
