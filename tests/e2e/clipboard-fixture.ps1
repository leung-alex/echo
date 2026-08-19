param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("copy-text", "copy-html", "copy-rtf", "copy-image", "copy-files", "copy-unsupported")]
    [string] $Operation,
    [string] $Value = "fixture"
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

function Set-ClipboardDataObject([System.Windows.Forms.DataObject] $Data) {
    [System.Windows.Forms.Clipboard]::SetDataObject($Data, $true)
}

switch ($Operation) {
    "copy-text" {
        [System.Windows.Forms.Clipboard]::SetText($Value)
    }
    "copy-html" {
        $markerStart = "<!--StartFragment-->"
        $markerEnd = "<!--EndFragment-->"
        $body = "<html><body>$markerStart<b>$Value</b>$markerEnd</body></html>"
        $headerTemplate = "Version:0.9`r`nStartHTML:{0:00000000}`r`nEndHTML:{1:00000000}`r`nStartFragment:{2:00000000}`r`nEndFragment:{3:00000000}`r`n"
        $headerLength = [Text.Encoding]::UTF8.GetByteCount(($headerTemplate -f 0, 0, 0, 0))
        $bodyBytes = [Text.Encoding]::UTF8.GetBytes($body)
        $startHtml = $headerLength
        $endHtml = $startHtml + $bodyBytes.Length
        $startFragment = $startHtml + [Text.Encoding]::UTF8.GetByteCount($body.Substring(0, $body.IndexOf($markerStart)))
        $endFragment = $startHtml + [Text.Encoding]::UTF8.GetByteCount($body.Substring(0, $body.IndexOf($markerEnd)))
        $html = $headerTemplate -f $startHtml, $endHtml, $startFragment, $endFragment
        $html += $body
        $data = [System.Windows.Forms.DataObject]::new()
        $data.SetData([System.Windows.Forms.DataFormats]::UnicodeText, $Value)
        $data.SetData("HTML Format", $html)
        Set-ClipboardDataObject $data
    }
    "copy-rtf" {
        $data = [System.Windows.Forms.DataObject]::new()
        $data.SetData([System.Windows.Forms.DataFormats]::UnicodeText, $Value)
        $data.SetData([System.Windows.Forms.DataFormats]::Rtf, "{\rtf1\ansi\b $Value\b0}")
        Set-ClipboardDataObject $data
    }
    "copy-image" {
        $bitmap = [System.Drawing.Bitmap]::new(24, 24)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.Clear([System.Drawing.Color]::CornflowerBlue)
            $graphics.FillRectangle([System.Drawing.Brushes]::Gold, 4, 4, 16, 16)
            $data = [System.Windows.Forms.DataObject]::new()
            $data.SetImage($bitmap)
            Set-ClipboardDataObject $data
        }
        finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }
    "copy-files" {
        $path = Join-Path ([IO.Path]::GetTempPath()) ("echo-clipboard-" + [Guid]::NewGuid().ToString("N") + ".txt")
        Set-Content -LiteralPath $path -Value $Value -Encoding UTF8
        $files = [Collections.Specialized.StringCollection]::new()
        [void] $files.Add($path)
        $data = [System.Windows.Forms.DataObject]::new()
        $data.SetFileDropList($files)
        Set-ClipboardDataObject $data
        Write-Output $path
    }
    "copy-unsupported" {
        $data = [System.Windows.Forms.DataObject]::new()
        $data.SetData("Echo.Unsupported", $Value)
        Set-ClipboardDataObject $data
    }
}
