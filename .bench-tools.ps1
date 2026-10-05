$ErrorActionPreference = 'Stop'

$tools = @(
  @{
    type = 'function'
    function = @{
      name        = 'get_weather'
      description = 'Get the current weather for a city.'
      parameters  = @{
        type       = 'object'
        properties = @{ city = @{ type = 'string'; description = 'City name' } }
        required   = @('city')
      }
    }
  }
)

$targets = @(
  @{ n = 'Qwen3.5-4B (llama.cpp 18080)'; p = 18080; m = 'local' },
  @{ n = 'MiniCPM5-1B (LM Studio 18082)'; p = 18082; m = 'minicpm5-1b' }
)

foreach ($t in $targets) {
  $b = @{
    model    = $t.m
    messages = @(
      @{ role = 'system'; content = 'You have tools. If you need current weather you MUST call get_weather. Do not guess.' },
      @{ role = 'user';   content = 'What is the weather in Beijing right now?' }
    )
    tools       = $tools
    tool_choice = 'auto'
    max_tokens  = 200
    temperature = 0
    stream      = $false
  } | ConvertTo-Json -Depth 12 -Compress

  Write-Output ('======== ' + $t.n + ' ========')
  try {
    $r   = Invoke-RestMethod -Uri "http://127.0.0.1:$($t.p)/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $b -TimeoutSec 300
    $msg = $r.choices[0].message
    $n   = 0
    if ($msg.tool_calls) { $n = $msg.tool_calls.Count }
    Write-Output ('finish_reason : ' + $r.choices[0].finish_reason)
    Write-Output ('tool_calls    : ' + $n)
    if ($n -gt 0) {
      foreach ($tc in $msg.tool_calls) { Write-Output ('  -> ' + $tc.function.name + '  args=' + $tc.function.arguments) }
    }
    $c = $msg.content
    if (-not $c) { $c = '(empty)' }
    Write-Output ('content       : ' + $c)
  } catch {
    Write-Output ('ERROR: ' + $_.Exception.Message)
  }
  Write-Output ''
}