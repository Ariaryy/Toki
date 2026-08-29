param(
    [string]$Type = "get_state",
    [Nullable[bool]]$Paused = $null,
    [Nullable[long]]$ForMs = $null
)

$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "toki", [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(2000)

$body = switch ($Type) {
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
