$ErrorActionPreference = 'Continue'
Set-Location 'D:\96_CoderWorld\quill'
$fail = 0
function Ck($code, $label) {
  if ($code -eq 0) { "  [OK] $label" } else { "  [FAIL] $label"; $script:fail = 1 }
}

"[gate] dependencies count"
$n = (Get-Content ui\web\package.json -Raw | ConvertFrom-Json).dependencies.PSObject.Properties.Name.Count
if ($n -eq 8) { "  [OK] dependencies = $n" } else { "  [FAIL] dependencies = $n (expected 8)"; $fail = 1 }

"[gate] forbidden deps"
foreach ($bad in @('antd','lucide-react')) {
  if ((Get-Content ui\web\package.json -Raw) -match $bad) { "  [FAIL] found $bad"; $fail = 1 }
  else { "  [OK] no $bad" }
}

"[gate] i18n"
$o = node .i18n-check.mjs 2>&1 | Out-String
$o.Trim().Split("`n") | Select-Object -Last 3 | ForEach-Object { "  $_" }
Ck $LASTEXITCODE ".i18n-check"

"[gate] library"
$o2 = node .library-check.mjs 2>&1 | Out-String
$o2.Trim().Split("`n") | Select-Object -Last 2 | ForEach-Object { "  $_" }
Ck $LASTEXITCODE ".library-check"

if ($fail -eq 0) { "`nGATES PASSED" } else { "`nGATES FAILED" }
