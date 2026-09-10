// Read-only native sampling; no process/working-set mutation. EX2 requires a
// September 2023 Windows 10/11 update. Unsupported systems fail closed.
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace Echo.Performance {
    public sealed class ProcessIdentity {
        public int ProcessId, ParentProcessId, SessionId;
        public string Name, ExecutablePath;
        public DateTime CreationDate;
        public int IdentityError;
    }
    public static class ProcessSnapshot {
        [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
        private struct Entry {
            public uint Size, Usage, ProcessId;
            public UIntPtr Heap;
            public uint ModuleId, Threads, ParentProcessId;
            public int BasePriority;
            public uint Flags;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst=260)] public string Name;
        }
        [StructLayout(LayoutKind.Sequential)]
        public struct Memory {
            public uint Size, PageFaults;
            public UIntPtr PeakWorkingSet, WorkingSet, PeakPagedPool, PagedPool,
                PeakNonPagedPool, NonPagedPool, PagefileUsage, PeakPagefileUsage,
                PrivateUsage, PrivateWorkingSet;
            public ulong SharedCommit;
        }
        [DllImport("kernel32.dll", SetLastError=true)] private static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool Process32FirstW(IntPtr snapshot, ref Entry entry);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool Process32NextW(IntPtr snapshot, ref Entry entry);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
        [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool ProcessIdToSessionId(uint pid, out uint session);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetProcessTimes(IntPtr handle, out long created, out long exited, out long kernel, out long user);
        [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool QueryFullProcessImageNameW(IntPtr handle, uint flags, StringBuilder name, ref uint size);
        [DllImport("kernel32.dll", SetLastError=true)] private static extern bool K32GetProcessMemoryInfo(IntPtr handle, ref Memory counters, uint size);

        public static ProcessIdentity[] Capture() {
            IntPtr snapshot = CreateToolhelp32Snapshot(2, 0);
            if (snapshot == new IntPtr(-1)) throw new Win32Exception();
            try {
                var rows = new List<ProcessIdentity>();
                var entry = new Entry { Size = (uint)Marshal.SizeOf(typeof(Entry)) };
                bool more = Process32FirstW(snapshot, ref entry);
                if (!more) throw new Win32Exception();
                while (more) {
                    var row = new ProcessIdentity { ProcessId=(int)entry.ProcessId,
                        ParentProcessId=(int)entry.ParentProcessId, Name=entry.Name,
                        CreationDate=DateTime.SpecifyKind(DateTime.MinValue, DateTimeKind.Utc) };
                    uint session;
                    if (ProcessIdToSessionId(entry.ProcessId, out session)) row.SessionId=(int)session;
                    else { row.SessionId=-1; row.IdentityError=Marshal.GetLastWin32Error(); }
                    IntPtr handle = OpenProcess(0x1000, false, row.ProcessId);
                    if (handle == IntPtr.Zero) row.IdentityError=Marshal.GetLastWin32Error();
                    else try {
                        long created, exited, kernel, user;
                        if (GetProcessTimes(handle, out created, out exited, out kernel, out user))
                            row.CreationDate=DateTime.FromFileTimeUtc(created);
                        else row.IdentityError=Marshal.GetLastWin32Error();
                        uint size=32768;
                        var name = new StringBuilder((int)size);
                        if (QueryFullProcessImageNameW(handle, 0, name, ref size)) row.ExecutablePath=name.ToString();
                        else row.IdentityError=Marshal.GetLastWin32Error();
                    } finally { CloseHandle(handle); }
                    rows.Add(row);
                    more = Process32NextW(snapshot, ref entry);
                }
                if (Marshal.GetLastWin32Error() != 18) throw new Win32Exception();
                return rows.ToArray();
            } finally { CloseHandle(snapshot); }
        }

        public static Memory ReadMemory(int pid, long expectedCreatedTicks) {
            IntPtr handle=OpenProcess(0x410, false, pid);
            if (handle == IntPtr.Zero) throw new Win32Exception();
            try {
                long created, exited, kernel, user;
                if (!GetProcessTimes(handle, out created, out exited, out kernel, out user)) throw new Win32Exception();
                if (DateTime.FromFileTimeUtc(created).Ticks != expectedCreatedTicks)
                    throw new InvalidOperationException("PID identity changed during native memory read.");
                var memory = new Memory { Size=(uint)Marshal.SizeOf(typeof(Memory)) };
                if (!K32GetProcessMemoryInfo(handle, ref memory, memory.Size)) throw new Win32Exception();
                return memory;
            } finally { CloseHandle(handle); }
        }
    }
}
