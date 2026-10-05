$ErrorActionPreference = 'Stop'

# 8 个工具，模拟 quill 的真实工具表规模；要求选对工具 + 复杂参数
$tools = @(
  @{ type='function'; function=@{ name='read_file'; description='Read a file from disk.'; parameters=@{ type='object'; properties=@{ path=@{type='string'} }; required=@('path') } } },
  @{ type='function'; function=@{ name='write_file'; description='Write content to a file.'; parameters=@{ type='object'; properties=@{ path=@{type='string'}; content=@{type='string'} }; required=@('path','content') } } },
  @{ type='function'; function=@{ name='list_dir'; description='List files in a directory.'; parameters=@{ type='object'; properties=@{ path=@{type='string'} }; required=@('path') } } },
  @{ type='function'; function=@{ name='run_command'; description='Run a shell command.'; parameters=@{ type='object'; properties=@{ cmd=@{type='string'} }; required=@('cmd') } } },
  @{ type='function'; function=@{ name='list_experts'; description='List available expert personas.'; parameters=@{ type='object'; properties=@{} } } },
  @{ type='function'; function=@{ name='get_expert_detail'; description='Get one expert detail.'; parameters=@{ type='object'; properties=@{ name=@{type='string'} }; required=@('name') } } },
  @{ type='function'; function=@{ name='text-parser'; description='Parse text into key-value pairs.'; parameters=@{ type='object'; properties=@{ task=@{type='string'}; input=@{type='string'} }; required=@('task','input') } } },
  @{ type='function'; function=@{ name='mesh-analysis'; description='Analyze a 3D mesh file.'; parameters=@{ type='object'; properties=@{ stl_path=@{type='string'}; output_format=@{type='string'} }; required=@('stl_path','output_format') } } }
)

$sys = 'You are an agent with tools. Always use a tool when one is needed. Do not fabricate results.'
$u1  = 'The file /app/data/mesh.stl needs to be analyzed. Produce the report as JSON. Do it.'

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
  $r = Post $t.p $t.m @(@{role='system';content=$sys}, @{role='user';content=$u1})
  $m1 = $r.choices[0].message
  $tc = $m1.tool_calls
  if ($tc -and $tc.Count -gt 0) {
    Write-Output ('step1 tool : ' + $tc[0].function.name)
    Write-Output ('step1 args : ' + $tc[0].function.arguments)
  } else {
    Write-Output 'step1      : NO TOOL CALL (直接回话了)'
    Write-Output ('step1 text : ' + $m1.content)
  }
  Write-Output ''
}