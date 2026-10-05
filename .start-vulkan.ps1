$ErrorActionPreference = 'Stop'
Get-Process -Id 11932 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Seconds 2

$exe  = 'D:\00_ProgramFiles\llama-b11146\llama-server.exe'
$gguf = 'D:\00_ProgramFiles\llama-b10068\models\Qwen3.5-4B-Q4_K_M.gguf'
$log  = 'D:\96_CoderWorld\quill\.llama-vulkan.log'
$err  = 'D:\96_CoderWorld\quill\.llama-vulkan.err'

$args = @(
  '-m', $gguf,
  '--host', '127.0.0.1',
  '--port', '18081',
  '-c', '8192',
  '-t', '14',
  '--reasoning', 'off',
  '-dev', 'Vulkan0',
  '-ngl', '99',
  '-np', '1',
  '-lv', '5'
)

$p = Start-Process -FilePath $exe -ArgumentList $args -RedirectStandardOutput $log `
     -RedirectStandardError $err -PassThru -WindowStyle Hidden
"started pid=" + $p.Id