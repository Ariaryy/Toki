param(
    [string]$Type = "get_state",
    [Nullable[bool]]$Paused = $null,
    [Nullable[long]]$ForMs = $null,
    # Raw JSON body, for message types below that don't have their own switch case yet
    # (set_app_alias, set_domain_alias, set_ignored, set_idle_threshold_ms). Overrides -Type.
    # Example: -Body '{"type":"set_ignored","exe_path":"C:\\foo.exe","ignored":true}'
    [string]$Body = $null
)

# The daemon names its pipe after the user's SID: one daemon per account.
$pipeName = "toki-" + [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(2000)

$body = if ($Body) { $Body } else {
    switch ($Type) {
        "get_state"   { '{"type":"get_state"}' }
        "resolve_now" { '{"type":"resolve_now"}' }
        "set_pause"   {
            if ($Paused) {
                if ($ForMs) { "{`"type`":`"set_pause`",`"paused`":true,`"for_ms`":$ForMs}" }
                else { '{"type":"set_pause","paused":true,"for_ms":null}' }
            } else {
                '{"type":"set_pause","paused":false}'
            }
        }
    }
}

$bytes = [System.Text.Encoding]::UTF8.GetBytes($body)
$len = [BitConverter]::GetBytes([uint32]$bytes.Length)
$pipe.Write($len, 0, 4)
$pipe.Write($bytes, 0, $bytes.Length)
$pipe.Flush()

$lenBuf = New-Object byte[] 4
$pipe.Read($lenBuf, 0, 4) | Out-Null
$rlen = [BitConverter]::ToUInt32($lenBuf, 0)
$respBuf = New-Object byte[] $rlen
$read = 0
while ($read -lt $rlen) {
    $read += $pipe.Read($respBuf, $read, $rlen - $read)
}
$resp = [System.Text.Encoding]::UTF8.GetString($respBuf)
Write-Output "sent: $body"
Write-Output "recv: $resp"
$pipe.Close()
