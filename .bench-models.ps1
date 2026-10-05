$ErrorActionPreference = 'Stop'

$tok = 'dev-token'
$sid = '9B95470E22BA2D9989B3506E773B1F4E'
$msgs = Invoke-RestMethod -Uri "http://127.0.0.1:8848/api/sessions/$sid/messages" -Headers @{ Authorization = "Bearer $tok" }
$long = ($msgs.messages | Where-Object { $_.role -eq 'user' } | Select-Object -First 1).content
$short = 'Say exactly: OK'

$servers = @(
  @{ n = 'Qwen3.5-4B  CPU (llama.cpp 18080)'; p = 18080; m = 'local' },
  @{ n = 'MiniCPM5-1B CPU (LM Studio 18082)'; p = 18082; m = 'minicpm5-1b' }
)

function Invoke-One($port, $model, $prompt, $maxTokens) {
  $b = @{ model = $model; messages = @(@{ role = 'user'; content = $prompt }); max_tokens = $maxTokens; temperature = 0; stream = $false } | ConvertTo-Json -Depth 6 -Compress
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $b -TimeoutSec 600
  $sw.Stop()
  return @{
    ms     = $sw.ElapsedMilliseconds
    ptok   = $r.usage.prompt_tokens
    ctok   = $r.usage.completion_tokens
    finish = $r.choices[0].finish_reason
    text   = $r.choices[0].message.content
  }
}

$rows = @()
foreach ($s in $servers) {
  # warm up
  $null = Invoke-One $s.p $s.m $short 8 | Out-Null

  $a = Invoke-One $s.p $s.m $long  64
  $b = Invoke-One $s.p $s.m $short 64

  $rows += [pscustomobject]@{
    server   = $s.n
    long_wall_ms   = $a.ms
    long_in_tok    = $a.ptok
    long_out_tok   = $a.ctok
    short_wall_ms  = $b.ms
    short_out_tok  = $b.ctok
    short_tok_s    = [math]::Round($b.ctok / ($b.ms / 1000.0), 2)
    sample = ($b.text -replace '\s+', ' ').Substring(0, [math]::Min(40, $b.text.Length))
  }
}

$rows | Format-Table -AutoSize -Wrap