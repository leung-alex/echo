$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class EchoTargetForeground {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool AllowSetForegroundWindow(uint processId);

    [DllImport("user32.dll")]
    private static extern bool BringWindowToTop(IntPtr window);

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool AttachThreadInput(uint attach, uint attachTo, bool attachInput);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    private static extern IntPtr SetFocus(IntPtr window);

    [DllImport("user32.dll")]
    private static extern void SwitchToThisWindow(IntPtr window, bool altTab);

    [DllImport("user32.dll")]
    private static extern bool GetCursorPos(out Point point);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr window, out Rect rect);

    [DllImport("user32.dll")]
    private static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    private static extern bool SetWindowPos(
        IntPtr window,
        IntPtr insertAfter,
        int x,
        int y,
        int width,
        int height,
        uint flags
    );

    [DllImport("user32.dll")]
    private static extern void mouse_event(uint flags, uint x, uint y, uint data, IntPtr extraInfo);

    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left; public int Top; public int Right; public int Bottom; }

    private static readonly IntPtr Topmost = new IntPtr(-1);
    private static readonly IntPtr NotTopmost = new IntPtr(-2);

    public static bool Activate(IntPtr window) {
        BringWindowToTop(window);
        SetForegroundWindow(window);
        if (GetForegroundWindow() == window) return true;
        SwitchToThisWindow(window, false);
        SetForegroundWindow(window);
        if (GetForegroundWindow() == window) return true;
        uint foregroundProcessId;
        uint foregroundThreadId = GetWindowThreadProcessId(GetForegroundWindow(), out foregroundProcessId);
        uint currentThreadId = GetCurrentThreadId();
        bool attached = foregroundThreadId != 0 && foregroundThreadId != currentThreadId &&
            AttachThreadInput(currentThreadId, foregroundThreadId, true);
        try {
            BringWindowToTop(window);
            SetForegroundWindow(window);
        } finally {
            if (attached) AttachThreadInput(currentThreadId, foregroundThreadId, false);
        }
        if (GetForegroundWindow() != window) {
            Point cursor;
            Rect bounds;
            if (GetCursorPos(out cursor) && GetWindowRect(window, out bounds)) {
                SetWindowPos(window, Topmost, 0, 0, 0, 0, 0x0002 | 0x0001 | 0x0010);
                SetCursorPos(bounds.Left + (bounds.Right - bounds.Left) / 2, bounds.Top + (bounds.Bottom - bounds.Top) / 2);
                mouse_event(0x0002, 0, 0, 0, IntPtr.Zero);
                mouse_event(0x0004, 0, 0, 0, IntPtr.Zero);
                SetWindowPos(window, NotTopmost, 0, 0, 0, 0, 0x0002 | 0x0001 | 0x0010);
                SetCursorPos(cursor.X, cursor.Y);
            }
        }
        return GetForegroundWindow() == window;
    }

    public static bool ForceFocus(IntPtr window) {
        return SetFocus(window) == window;
    }

}
"@

foreach ($path in @(
    $env:ECHO_TARGET_READY,
    $env:ECHO_TARGET_COMMAND,
    $env:ECHO_TARGET_RESPONSE,
    $env:ECHO_TARGET_PRIMARY_OUTPUT,
    $env:ECHO_TARGET_SECONDARY_OUTPUT,
    $env:ECHO_TARGET_PASSWORD_OUTPUT
)) {
    if (-not $path -or -not [IO.Path]::IsPathRooted($path)) {
        throw "Echo target fixture paths must be absolute"
    }
}
if (-not $env:ECHO_TARGET_RUN_ID) {
    throw "Echo target fixture run id is missing"
}

function Write-AtomicText($path, $value) {
    $temporary = "$path.tmp"
    [IO.File]::WriteAllText($temporary, [string]$value, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $path -Force
}

function Focus-Control($control, $caret = $null) {
    $foreground = [EchoTargetForeground]::GetForegroundWindow()
    [uint32]$foregroundProcessId = 0
    [uint32]$foregroundThreadId = if ($foreground -eq [IntPtr]::Zero) { 0 } else {
        [EchoTargetForeground]::GetWindowThreadProcessId(
            $foreground,
            [ref]$foregroundProcessId
        )
    }
    [uint32]$currentThreadId = [EchoTargetForeground]::GetCurrentThreadId()
    $attached = $foregroundThreadId -ne 0 -and $foregroundThreadId -ne $currentThreadId
    if ($attached -and -not [EchoTargetForeground]::AttachThreadInput(
        $currentThreadId,
        $foregroundThreadId,
        $true
    )) {
        throw "Echo target fixture foreground authorization failed"
    }
    try {
        [void]$window.Activate()
        $handle = $window.Handle
        if (-not [EchoTargetForeground]::Activate($handle)) {
            throw "Echo target fixture could not become foreground"
        }
        [void]$control.Focus()
        [void][EchoTargetForeground]::ForceFocus($control.Handle)
    }
    finally {
        if ($attached -and -not [EchoTargetForeground]::AttachThreadInput(
            $currentThreadId,
            $foregroundThreadId,
            $false
        )) {
            throw "Echo target fixture foreground cleanup failed"
        }
    }
    [void][EchoTargetForeground]::ForceFocus($control.Handle)
    if ($null -ne $caret) {
        $control.SelectionStart = [int]$caret
        $control.SelectionLength = 0
    }
}

$window = [Windows.Forms.Form]::new()
$window.Text = $env:ECHO_TARGET_TITLE
$window.Width = 520
$window.Height = 280
$window.StartPosition = "CenterScreen"

$panel = [Windows.Forms.FlowLayoutPanel]::new()
$panel.Dock = "Fill"
$panel.FlowDirection = "TopDown"
$panel.WrapContents = $false
$primary = [Windows.Forms.TextBox]::new()
$primary.Text = "ac"
$primary.Width = 450
$primary.Height = 44
$secondary = [Windows.Forms.TextBox]::new()
$secondary.Width = 450
$secondary.Height = 44
$password = [Windows.Forms.TextBox]::new()
$password.Width = 450
$password.Height = 44
$password.UseSystemPasswordChar = $true
[void]$panel.Controls.Add($primary)
[void]$panel.Controls.Add($secondary)
[void]$panel.Controls.Add($password)
[void]$window.Controls.Add($panel)

Write-AtomicText $env:ECHO_TARGET_PRIMARY_OUTPUT $primary.Text
Write-AtomicText $env:ECHO_TARGET_SECONDARY_OUTPUT ""
Write-AtomicText $env:ECHO_TARGET_PASSWORD_OUTPUT ""
$primary.Add_TextChanged({ Write-AtomicText $env:ECHO_TARGET_PRIMARY_OUTPUT $primary.Text })
$secondary.Add_TextChanged({ Write-AtomicText $env:ECHO_TARGET_SECONDARY_OUTPUT $secondary.Text })
$password.Add_TextChanged({ Write-AtomicText $env:ECHO_TARGET_PASSWORD_OUTPUT $password.Text })

$window.Add_Shown({
    Focus-Control $primary 1
    $ready = [ordered]@{
        run_id = $env:ECHO_TARGET_RUN_ID
        process_id = $PID
        title = $window.Text
    } | ConvertTo-Json -Compress
    Write-AtomicText $env:ECHO_TARGET_READY $ready
})

$timer = [Windows.Forms.Timer]::new()
$timer.Interval = 25
$timer.Add_Tick({
    if (-not (Test-Path -LiteralPath $env:ECHO_TARGET_COMMAND)) {
        return
    }
    $request = [IO.File]::ReadAllText($env:ECHO_TARGET_COMMAND) | ConvertFrom-Json
    Remove-Item -LiteralPath $env:ECHO_TARGET_COMMAND -Force
    $value = ""
    $errorMessage = ""
    try {
        if ($request.run_id -ne $env:ECHO_TARGET_RUN_ID -or -not $request.request_id) {
            throw "Echo target fixture command ownership is invalid"
        }
        switch ([string]$request.command) {
            "allow-foreground" {
                [uint32]$echoPid = 0
                if (-not [uint32]::TryParse([string]$request.payload, [ref]$echoPid) -or $echoPid -eq 0) {
                    throw "Echo process id is invalid"
                }
                $value = [EchoTargetForeground]::AllowSetForegroundWindow($echoPid).ToString().ToLowerInvariant()
            }
            "focus-primary" { Focus-Control $primary 1 }
            "focus-secondary" { Focus-Control $secondary 0 }
            "focus-password" { Focus-Control $password }
            "primary-focused" { $value = ([Windows.Forms.Form]::ActiveForm -eq $window -and $primary.Focused).ToString().ToLowerInvariant() }
            "read-primary" { $value = $primary.Text }
            "read-secondary" { $value = $secondary.Text }
            "read-password" { $value = $password.Text }
            "copy-password" {
                Focus-Control $password
                [Windows.Forms.Clipboard]::SetText("password-secret")
            }
            "shutdown" {
                $timer.Stop()
                $window.Close()
            }
            default { throw "Unknown Echo target fixture command" }
        }
    }
    catch {
        $errorMessage = $_.Exception.Message
    }
    $response = [ordered]@{
        request_id = $request.request_id
        value = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($value))
        error = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($errorMessage))
    } | ConvertTo-Json -Compress
    Write-AtomicText $env:ECHO_TARGET_RESPONSE $response
})
$timer.Start()
[void]$window.ShowDialog()
