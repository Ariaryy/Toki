# Export installed app icons for /?busy captures without reading activity or browser data.
# Usage: ./scripts/export-preview-icons.ps1 -OutputPath .shots/app-icons.json
# Then set TOKI_PREVIEW_ICONS to that JSON path before starting preview-gui.py.
param([string]$OutputPath = ".shots/app-icons.json")
$ErrorActionPreference = "Stop"
if (-not [IO.Path]::IsPathRooted($OutputPath)) {
    $OutputPath = Join-Path (Split-Path $PSScriptRoot -Parent) $OutputPath
}
Add-Type -AssemblyName System.Drawing
$icons = @{}
foreach ($exe in (Get-Process | Where-Object Path | Select-Object -ExpandProperty Path -Unique)) {
    $name = [IO.Path]::GetFileName($exe).ToLowerInvariant()
    if ($icons.ContainsKey($name)) { continue }
    try {
        $icon = [Drawing.Icon]::ExtractAssociatedIcon($exe)
        if ($null -eq $icon) { continue }
        $bitmap = $icon.ToBitmap()
        $stream = [IO.MemoryStream]::new()
        try {
            $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
            $icons[$name] = "data:image/png;base64," + [Convert]::ToBase64String($stream.ToArray())
        } finally {
            $stream.Dispose()
            $bitmap.Dispose()
            $icon.Dispose()
        }
    } catch {
        Write-Verbose "No readable icon for $name"
    }
}
$icons | ConvertTo-Json | Set-Content -LiteralPath $OutputPath -Encoding utf8
Write-Host "Exported $($icons.Count) installed icons to $OutputPath"
