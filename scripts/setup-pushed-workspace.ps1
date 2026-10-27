param(
    [string]$WorkspaceCommit = "HEAD",
    [string]$TargetRef = "refs/tags/0.22.3-base",
    [string]$PushRemote = "xj-ms",
    [string]$TargetCommitId = ""
)

$ErrorActionPreference = "Stop"

$wsSha = (git --no-pager rev-parse --verify "$WorkspaceCommit^{commit}").Trim()
$parents = @(git --no-pager rev-parse "$wsSha^@" | ForEach-Object { $_.Trim() } | Where-Object { $_ })

if ($parents.Count -eq 0) {
    throw "Commit $wsSha has no parents."
}

$lines = @(git --no-pager log -1 --format=%B $wsSha)
$branches = @(
    foreach ($line in $lines) {
        if ($line -match '^\s*-\s+(.+)$') {
            $Matches[1].Trim()
        }
    }
)

if ($branches.Count -ne $parents.Count) {
    throw "Found $($branches.Count) branch names in commit message, but commit $wsSha has $($parents.Count) parents."
}

git --no-pager show-ref --verify --quiet $TargetRef
if ($LASTEXITCODE -ne 0) {
    throw "Target ref '$TargetRef' does not exist."
}

if (-not $TargetCommitId) {
    $TargetCommitId = (git --no-pager rev-parse --verify "$TargetRef^{commit}").Trim()
}

Write-Host "Workspace commit : $wsSha"
Write-Host "Target ref       : $TargetRef"
Write-Host "Target commit    : $TargetCommitId"
Write-Host "Push remote      : $PushRemote"
Write-Host "Virtual branches : $($branches.Count)"

# 1. Create/update local branch refs for each parent of the workspace commit
for ($i = 0; $i -lt $branches.Count; $i++) {
    $branch = $branches[$i]
    $parentSha = $parents[$i]
    git update-ref "refs/heads/$branch" $parentSha

    $remoteRef = "refs/remotes/$PushRemote/$branch"
    git show-ref --verify --quiet $remoteRef
    if ($LASTEXITCODE -eq 0) {
        git config "branch.$branch.remote" $PushRemote
        git config "branch.$branch.merge" "refs/heads/$branch"
    }
    Write-Host ("  [{0,2}] {1} -> {2}" -f $i, $branch, $parentSha.Substring(0, 10))
}

# 2. Configure GitButler project metadata in local git config
git config --local gitbutler.project.targetRef $TargetRef
git config --local gitbutler.project.targetCommitId $TargetCommitId
git config --local gitbutler.project.pushRemote $PushRemote
git config --local gitbutler.project.portedMeta true

# 3. Point refs/heads/gitbutler/workspace at the workspace commit and switch HEAD without touching the worktree
git update-ref refs/heads/gitbutler/workspace $wsSha
git symbolic-ref HEAD refs/heads/gitbutler/workspace

# 4. Register the repository in GitButler's project registry first (prefer repo-built target\debug\but.exe)
$localBut = Join-Path (Resolve-Path (Join-Path $PSScriptRoot "..")).Path "target\debug\but.exe"
if (Test-Path $localBut) {
    & $localBut setup | Out-Null
} elseif (Get-Command but -ErrorAction SilentlyContinue) {
    but setup | Out-Null
}

# 5. Write .git/gitbutler/virtual_branches.toml (including [default_target] for 0.22.x compatibility)
$gitDir = (Resolve-Path (git --no-pager rev-parse --git-dir).Trim()).Path
$gbDir = Join-Path $gitDir "gitbutler"
New-Item -ItemType Directory -Force -Path $gbDir | Out-Null

if ($TargetRef -like "refs/tags/*") {
    $targetRemote = ""
    $targetBranch = $TargetRef -replace '^refs/tags/', ''
} else {
    $targetParts = ($TargetRef -replace '^refs/remotes/', '') -split '/', 2
    $targetRemote = $targetParts[0]
    $targetBranch = $targetParts[1]
}

$sb = [System.Text.StringBuilder]::new()
[void]$sb.AppendLine("[default_target]")
[void]$sb.AppendLine("branchName = `"$targetBranch`"")
[void]$sb.AppendLine("remoteName = `"$targetRemote`"")
[void]$sb.AppendLine("remoteUrl = `"`"")
[void]$sb.AppendLine("sha = `"$TargetCommitId`"")
[void]$sb.AppendLine("pushRemoteName = `"$PushRemote`"")
[void]$sb.AppendLine()

for ($i = 0; $i -lt $branches.Count; $i++) {
    $id = [guid]::NewGuid().ToString()
    $branch = $branches[$i]
    $parentSha = $parents[$i]

    [void]$sb.AppendLine("[branches.$id]")
    [void]$sb.AppendLine("id = `"$id`"")
    [void]$sb.AppendLine("order = $i")
    [void]$sb.AppendLine("in_workspace = true")
    [void]$sb.AppendLine("notes = `"`"")
    [void]$sb.AppendLine("ownership = `"`"")
    [void]$sb.AppendLine("allow_rebasing = true")
    [void]$sb.AppendLine("post_commits = false")
    [void]$sb.AppendLine("tree = `"0000000000000000000000000000000000000000`"")
    [void]$sb.AppendLine("created_timestamp_ms = `"0`"")
    [void]$sb.AppendLine("updated_timestamp_ms = `"0`"")
    [void]$sb.AppendLine("name = `"$branch`"")
    [void]$sb.AppendLine("head = `"$parentSha`"")
    [void]$sb.AppendLine()
    [void]$sb.AppendLine("[[branches.$id.heads]]")
    [void]$sb.AppendLine("name = `"$branch`"")
    [void]$sb.AppendLine("archived = false")
    [void]$sb.AppendLine("[branches.$id.heads.head]")
    [void]$sb.AppendLine("CommitId = `"$parentSha`"")
    [void]$sb.AppendLine()
}

$tomlPath = Join-Path $gbDir "virtual_branches.toml"
[System.IO.File]::WriteAllText($tomlPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))

# Remove any existing SQLite cache so GitButler imports the fresh virtual_branches.toml on next open
Get-ChildItem -Path $gbDir -Filter "but.sqlite*" -ErrorAction SilentlyContinue | Remove-Item -Force

Write-Host "`nDone! HEAD is now on gitbutler/workspace with $($branches.Count) virtual branches."
