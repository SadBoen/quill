$ErrorActionPreference = 'Stop'

$tok = 'dev-token'
$sid = '9B95470E22BA2D9989B3506E773B1F4E'
$msgs = Invoke-RestMethod -Uri "http://127.0.0.1:8848/api/sessions/$sid/messages" -Headers @{ Authorization = "Bearer $tok" }
$prompt = ($msgs.messages | Where-Object { $_.role -eq 'user' } | Select-Object -First 1).content
Write-Output ("prompt_chars=" + $prompt.Length)

$targets = @(
  @{ name = 'CPU  (18080, existing)';  port = 18080; slots = '4' },
  @{ name = 'GPU  (18081, Vulkan 33/33)'; port = 18081; slots = '1' }
)

$body = @{
  model = 'local'
  messages = @(
    @{ role = 'system'; content = 'You are a helpful assistant. Answer briefly.' },
    @{ role = 'user';   content = $prompt }
  )
  max_tokens = 128
  temperature = 0
  stream = $false
} | ConvertTo-Json -Depth 8 -Compress

$results = @()
foreach ($t in $targets) {
  $null = Invoke-RestMethod -Uri "http://127.0.0.1:$($t.port)/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 300

  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $r  = Invoke-RestMethod -Uri "http://127.0.0.1:$($t.port)/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 300
  $sw.Stop()

  $tm = $r.timings
  $results += [pscustomobject]@{
    mode            = $t.name
    slots           = $t.slots
    prompt_ms       = [math]::Round($tm.prompt_ms)
    prompt_tok_s    = [math]::Round($tm.prompt_per_second, 1)
    predicted_ms    = [math]::Round($tm.predicted_ms)
    predicted_tok_s = [math]::Round($tm.predicted_per_second, 2)
    total_ms        = [math]::Round($sw.ElapsedMilliseconds)
    prompt_n        = $tm.prompt_n
    predicted_n     = $tm.predicted_n
  }
}

$results | Format-Table -AutoSize