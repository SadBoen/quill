$ErrorActionPreference = 'Stop'

$tools = @(
  @{ type='function'; function=@{ name='read_file'; description='Read a file from disk.'; parameters=@{ type='object'; properties=@{ path=@{type='string'} }; required=@('path') } } },
  @{ type='function'; function=@{ name='mesh-analysis'; description='Analyze a 3D mesh file.'; parameters=@{ type='object'; properties=@{ stl_path=@{type='string'}; output_format=@{type='string'} }; required=@('stl_path','output_format') } } },
  @{ type='function'; function=@{ name='list_experts'; description='List available expert personas.'; parameters=@{ type='object'; properties=@{} } } }
)

$sys = 'You are an agent with tools. When you already have the result you need, answer in plain text. Do not call tools again.'
$u1  = 'The file /app/data/mesh.stl needs to be analyzed. Report the triangle count.'

# 模拟服务端已经跑完工具并把结果喂回去
$toolResult = '{"triangles": 2048, "vertices": 1030, "watertight": false, "format": "json"}'

function Post($port, $model, $messages) {
  $b = @{ model=$model; messages=$messages; tools=$tools; tool_choice='auto'; max_tokens=300; temperature=0; stream=$false } | ConvertTo-Json -Depth 14 -Compress
  Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/chat/completions" -Method Post -ContentType 'application/json' -Body $b -TimeoutSec 300
}

$targets = @(
  @{ n='Qwen3.5-4B'; p=18080; m='local' },
  @{ n='MiniCPM5-1B'; p=18082; m='minicpm5-1b' }
)

foreach ($t in $targets) {
  Write-Output ('======== ' + $t.n + ' ========')
  $sw = [System.Diagnostics.Stopwatch]::StartNew()

  $msgs = @(
    @{ role='system'; content=$sys },
    @{ role='user'; content=$u1 },
    @{ role='assistant'; content=''; tool_calls=@(
        @{ id='call_1'; type='function'; function=@{ name='mesh-analysis'; arguments='{"stl_path":"/app/data/mesh.stl","output_format":"json"}' } }) },
    @{ role='tool'; name='mesh-analysis'; content=$toolResult }
  )

  $r = Post $t.p $t.m $msgs
  $sw.Stop()
  $msg = $r.choices[0].message
  $n = 0
  if ($msg.tool_calls) { $n = $msg.tool_calls.Count }
  Write-Output ('finish_reason : ' + $r.choices[0].finish_reason)
  Write-Output ('又调工具了吗   : ' + $n + ($(if ($n -gt 0) { ' -> ' + $msg.tool_calls[0].function.name } else { '' })))
  $c = $msg.content
  if (-not $c) { $c = '(empty)' }
  Write-Output ('正文           : ' + ($c -replace '\s+',' '))
  Write-Output ('耗时ms         : ' + $sw.ElapsedMilliseconds)
  Write-Output ''
}