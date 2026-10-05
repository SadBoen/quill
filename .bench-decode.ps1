$ErrorActionPreference = 'Stop'

# 让两个模型都产出足够多的 token，才能公平比解码速度
$gen = 'Count from 1 to 120, separated by commas, nothing else.'

$servers = @(
  @{ n = 'Qwen3.5-4B  (llama.cpp, CPU)'; p = 18080; m = 'local' },
  @{ n = 'MiniCPM5-1B (LM Studio, CPU)'; p = 18082; m = 'minicpm5-1b' }
)

function Run($port, $model, $prompt, $maxTokens) {
  $b = @{ model = $model; messages = @(@{ role = 'user'; content = $prompt }); max_tokens = $maxTokens; temperature = 0; stream = $false } | ConvertTo-Json -Depth 6 -Compress
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $r = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $b -TimeoutSec 600
  $sw.Stop()
  return @{ ms = $sw.ElapsedMilliseconds; ptok = $r.usage.prompt_tokens; ctok = $r.usage.completion_tokens; fin = $r.choices[0].finish_reason }
}

$rows = @()
foreach ($s in $servers) {
  $null = Run $s.p $s.m $gen 16 | Out-Null     # warm up
  $w = Run $s.p $s.m $gen 400                  # warm
  $r = Run $s.p $s.m $gen 400                  # measured
  $rows += [pscustomobject]@{
    server       = $s.n
    in_tok       = $r.ptok
    out_tok      = $r.ctok
    finish       = $r.fin
    wall_ms      = $r.ms
    decode_tok_s = [math]::Round($r.ctok / ($r.ms / 1000.0), 2)
  }
}
$rows | Format-Table -AutoSize