$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$repo = 'mdsmithson/rectangle'
$github = "https://github.com/$repo.git"
$headers = @{ 'User-Agent' = 'Rectangle-Updater' }

if ($env:RECTANGLE_UPDATE_MODE -eq 'install') {
    $target = $env:RECTANGLE_UPDATE_TARGET
    $candidate = $env:RECTANGLE_UPDATE_CANDIDATE
    $log = Join-Path (Split-Path $candidate) 'install-error.txt'
    $suffix = [Guid]::NewGuid().ToString('N')
    $staged = "$target.$suffix.next"
    $backup = "$target.$suffix.bak"
    $processId = [int]$env:RECTANGLE_UPDATE_PID
    $canRestart = $false
    $params = @{ FilePath = $target; WorkingDirectory = $env:RECTANGLE_UPDATE_CWD }
    if ($env:RECTANGLE_UPDATE_ARGS) { $params.ArgumentList = $env:RECTANGLE_UPDATE_ARGS }
    try {
        $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
        if ($process -and !$process.WaitForExit(60000)) { throw 'Rectangle did not exit in time.' }
        $canRestart = $true
        Copy-Item -LiteralPath $candidate -Destination $staged
        [IO.File]::Replace($staged, $target, $backup)
        try {
            Start-Process @params
        } catch {
            [IO.File]::Replace($backup, $target, $null)
            throw
        }
        Remove-Item -LiteralPath $backup -ErrorAction SilentlyContinue
    } catch {
        $_.Exception.Message | Set-Content -LiteralPath $log -Encoding UTF8
        if ($canRestart -and (Test-Path -LiteralPath $target -PathType Leaf)) {
            try { Start-Process @params } catch { }
        }
        Add-Type -AssemblyName PresentationFramework
        [void][Windows.MessageBox]::Show($_.Exception.Message, 'Rectangle could not install the update')
    } finally {
        Remove-Item -LiteralPath $staged, $candidate -ErrorAction SilentlyContinue
    }
    exit
}

function Read-Version([string]$manifest) {
    $match = [regex]::Match($manifest, '(?m)^version\s*=\s*"((?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))"\s*$')
    if (!$match.Success) { throw 'The update does not declare a supported version.' }
    return $match.Groups[1].Value
}
if ($env:RECTANGLE_UPDATE_MODE -eq 'check') {
    $commit = (Invoke-RestMethod -TimeoutSec 15 -Headers $headers -Uri "https://api.github.com/repos/$repo/commits/main").sha
    if ($commit -cnotmatch '^[a-f0-9]{40}$') { throw 'GitHub did not return a valid commit.' }
    $manifest = (Invoke-WebRequest -UseBasicParsing -TimeoutSec 15 -Headers $headers -Uri "https://raw.githubusercontent.com/$repo/$commit/Cargo.toml").Content
    Write-Output "$commit $(Read-Version $manifest)"
    exit
}
if ($env:RECTANGLE_UPDATE_MODE -ne 'build') { throw 'Unknown update operation.' }
$sha = $env:RECTANGLE_UPDATE_SHA
if ($sha -cnotmatch '^[a-f0-9]{40}$') { throw 'Invalid update commit.' }
foreach ($tool in @('git.exe', 'cargo.exe')) {
    if (!(Get-Command $tool -CommandType Application -ErrorAction SilentlyContinue)) {
        throw "Install $tool before updating. The Windows build also requires MSVC Build Tools and a Windows SDK."
    }
}
$root = $env:RECTANGLE_UPDATE_DIR
$src = Join-Path $root 'source'
$env:GIT_TERMINAL_PROMPT = '0'
function Run-Git {
    & git.exe -c core.hooksPath=NUL @args
    if ($LASTEXITCODE -ne 0) { throw 'Could not fetch the update from GitHub.' }
}
Run-Git init --quiet $src
Run-Git -C $src fetch --quiet --depth=1 $github $sha
Run-Git -C $src checkout --quiet --detach $sha
$manifest = Join-Path $src 'Cargo.toml'
if ((Read-Version (Get-Content -LiteralPath $manifest -Raw)) -cne $env:RECTANGLE_UPDATE_VERSION) {
    throw 'The fetched source version did not match the update.'
}
Push-Location $src
try {
    $env:CARGO_TARGET_DIR = Join-Path $root 'target'
    & cargo.exe build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'The update did not build. The installed version was kept.' }
} finally {
    Pop-Location
}
$exe = Join-Path $env:CARGO_TARGET_DIR 'release/rectangle.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) { throw 'The build did not produce rectangle.exe.' }
Copy-Item -LiteralPath $exe -Destination (Join-Path $root 'rectangle.exe')
