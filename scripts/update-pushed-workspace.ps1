param(
    [string]$WorkspaceCommit = "",
    [string]$PushRemote = "",
    [string]$TargetRef = "",
    [string]$TargetCommitId = "",
    [switch]$Fetch,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"

# 1. Fail if there are uncommitted changes in tracked files
$dirtyLines = @(git --no-pager status --porcelain --untracked-files=no | Where-Object { $_ })
if ($dirtyLines.Count -gt 0) {
    [Console]::Error.WriteLine("Error: Cannot update workspace because the working tree has uncommitted changes:")
    foreach ($line in $dirtyLines) {
        [Console]::Error.WriteLine("  $line")
    }
    exit 1
}

# 2. Resolve push remote
if (-not $PushRemote) {
    $PushRemote = (git --no-pager config --local --get "branch.gitbutler/workspace.remote" 2>$null)
    if ($PushRemote) { $PushRemote = $PushRemote.Trim() }
}
if (-not $PushRemote) {
    $PushRemote = (git --no-pager config --local --get "gitbutler.project.pushRemote" 2>$null)
    if ($PushRemote) { $PushRemote = $PushRemote.Trim() }
}
if (-not $PushRemote) {
    $PushRemote = "origin"
}

if ($Fetch) {
    Write-Host "Fetching from '$PushRemote'..."
    git --no-pager fetch $PushRemote
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to fetch from remote '$PushRemote'."
    }
}

# 3. Resolve upstream workspace commit
$workspaceMergeRef = (git --no-pager config --local --get "branch.gitbutler/workspace.merge" 2>$null)
if ($workspaceMergeRef) { $workspaceMergeRef = $workspaceMergeRef.Trim() }
$workspaceRemoteShort = if ($workspaceMergeRef) { $workspaceMergeRef -replace '^refs/heads/', '' } else { "fork-master" }
$workspaceRemoteRef = "refs/remotes/$PushRemote/$workspaceRemoteShort"

if (-not $WorkspaceCommit) {
    $upstreamRef = (git --no-pager rev-parse --symbolic-full-name "refs/heads/gitbutler/workspace@{upstream}" 2>$null)
    if ($LASTEXITCODE -eq 0 -and $upstreamRef) {
        $WorkspaceCommit = $upstreamRef.Trim()
    } else {
        git --no-pager show-ref --verify --quiet $workspaceRemoteRef
        if ($LASTEXITCODE -eq 0) {
            $WorkspaceCommit = $workspaceRemoteRef
        } else {
            throw "Could not determine upstream ref for gitbutler/workspace. Pass -WorkspaceCommit explicitly."
        }
    }
}

$wsSha = (git --no-pager rev-parse --verify "$WorkspaceCommit^{commit}" 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $wsSha) {
    throw "Workspace commit '$WorkspaceCommit' could not be resolved."
}
$wsSha = $wsSha.Trim()

$currentWsSha = (git --no-pager rev-parse --verify "refs/heads/gitbutler/workspace^{commit}" 2>$null)
if ($LASTEXITCODE -eq 0 -and $currentWsSha) {
    $currentWsSha = $currentWsSha.Trim()
} else {
    $currentWsSha = (git --no-pager rev-parse --verify "HEAD^{commit}").Trim()
}

# 4. Resolve parent commits and map each parent to its branch name
$parents = @(git --no-pager rev-parse "$wsSha^@" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($parents.Count -eq 0) {
    throw "Workspace commit $wsSha has no parents."
}

# Build map of SHA -> remote branch names on $PushRemote (excluding HEAD and the workspace remote branch)
$remoteBranchesBySha = @{}
$remotePrefix = "refs/remotes/$PushRemote/"
git --no-pager for-each-ref --format="%(objectname) %(refname)" $remotePrefix | ForEach-Object {
    $parts = $_ -split ' ', 2
    $sha = $parts[0].Trim()
    $ref = $parts[1].Trim()
    if ($ref -eq "$remotePrefix`HEAD" -or $ref -eq $workspaceRemoteRef -or $ref -eq $WorkspaceCommit) {
        return
    }
    $shortName = $ref.Substring($remotePrefix.Length)
    if (-not $remoteBranchesBySha.ContainsKey($sha)) {
        $remoteBranchesBySha[$sha] = [System.Collections.Generic.List[string]]::new()
    }
    $remoteBranchesBySha[$sha].Add($shortName)
}

# Parse commit message for "- <branch>" and optional "branch head: <sha>" pairs as fallback/tie-breaker
$msgLines = @(git --no-pager log -1 --format=%B $wsSha)
$msgBranchesInOrder = [System.Collections.Generic.List[string]]::new()
$msgBranchBySha = @{}
$lastMsgBranch = $null
foreach ($line in $msgLines) {
    if ($line -match '^\s*-\s+(.+?)(?:\s+\(refs/heads/.+\))?\s*$') {
        $lastMsgBranch = $Matches[1].Trim()
        $msgBranchesInOrder.Add($lastMsgBranch)
    } elseif ($lastMsgBranch -and $line -match '^\s*branch head:\s*([0-9a-f]{7,40})\s*$') {
        $msgBranchBySha[$Matches[1].Trim()] = $lastMsgBranch
        $lastMsgBranch = $null
    }
}

$usedBranches = [System.Collections.Generic.HashSet[string]]::new()
$branches = [System.Collections.Generic.List[string]]::new()

for ($i = 0; $i -lt $parents.Count; $i++) {
    $parentSha = $parents[$i]
    $resolved = $null

    if ($remoteBranchesBySha.ContainsKey($parentSha)) {
        $candidates = @($remoteBranchesBySha[$parentSha] | Where-Object { -not $usedBranches.Contains($_) })
        if ($candidates.Count -eq 1) {
            $resolved = $candidates[0]
        } elseif ($candidates.Count -gt 1) {
            if ($msgBranchBySha.ContainsKey($parentSha) -and ($candidates -contains $msgBranchBySha[$parentSha])) {
                $resolved = $msgBranchBySha[$parentSha]
            } else {
                $resolved = $candidates[0]
            }
        }
    }

    if (-not $resolved -and $msgBranchBySha.ContainsKey($parentSha)) {
        $candidate = $msgBranchBySha[$parentSha]
        if (-not $usedBranches.Contains($candidate)) {
            $resolved = $candidate
        }
    }

    if (-not $resolved -and $msgBranchesInOrder.Count -eq $parents.Count) {
        $candidate = $msgBranchesInOrder[$i]
        if (-not $usedBranches.Contains($candidate)) {
            $resolved = $candidate
        }
    }

    if (-not $resolved) {
        throw "Could not resolve branch name for parent [$i] ($parentSha) of workspace commit $wsSha."
    }

    [void]$usedBranches.Add($resolved)
    $branches.Add($resolved)
}

# 5. Resolve target commit and target ref
$oldTargetRef = (git --no-pager config --local --get "gitbutler.project.targetRef" 2>$null)
if ($oldTargetRef) { $oldTargetRef = $oldTargetRef.Trim() }
$oldTargetCommitId = (git --no-pager config --local --get "gitbutler.project.targetCommitId" 2>$null)
if ($oldTargetCommitId) { $oldTargetCommitId = $oldTargetCommitId.Trim() }

if (-not $TargetCommitId) {
    if ($parents.Count -ge 2) {
        $octoBase = (git --no-pager merge-base --octopus @parents 2>$null)
        if ($LASTEXITCODE -eq 0 -and $octoBase) {
            $TargetCommitId = $octoBase.Trim()
        }
    }
    if (-not $TargetCommitId -and $TargetRef) {
        $TargetCommitId = (git --no-pager rev-parse --verify "$TargetRef^{commit}").Trim()
    }
    if (-not $TargetCommitId -and $oldTargetCommitId) {
        $TargetCommitId = $oldTargetCommitId
    }
    if (-not $TargetCommitId) {
        throw "Could not determine TargetCommitId. Pass -TargetCommitId explicitly."
    }
}

if (-not $TargetRef) {
    $tagAtBase = @(git --no-pager tag --points-at $TargetCommitId 2>$null | Where-Object { $_ })
    if ($tagAtBase.Count -gt 0) {
        $oldTagShort = if ($oldTargetRef -like "refs/tags/*") { $oldTargetRef -replace '^refs/tags/', '' } else { "" }
        $matchedTag = if ($oldTagShort -and ($tagAtBase -contains $oldTagShort)) { $oldTagShort } else { $tagAtBase[0] }
        $TargetRef = "refs/tags/$matchedTag"
    } elseif ($oldTargetRef) {
        $TargetRef = $oldTargetRef
    } else {
        $TargetRef = "refs/remotes/$PushRemote/master"
    }
}

$targetRefOldSha = (git --no-pager rev-parse --verify "$TargetRef^{commit}" 2>$null)
if ($LASTEXITCODE -eq 0 -and $targetRefOldSha) {
    $targetRefOldSha = $targetRefOldSha.Trim()
} else {
    $targetRefOldSha = ""
}
$updateTargetRef = ($targetRefOldSha -ne $TargetCommitId)

# 6. Print summary and ask for confirmation
$wsOldShort = if ($currentWsSha) { $currentWsSha.Substring(0, 10) } else { "(none)" }
$wsNewShort = $wsSha.Substring(0, 10)
$wsStatus = if ($currentWsSha -eq $wsSha) { "$wsNewShort (already at upstream)" } else { "$wsOldShort -> $wsNewShort" }
$targetRefDisplay = if ($oldTargetRef -and $oldTargetRef -ne $TargetRef) { "$oldTargetRef -> $TargetRef" } else { $TargetRef }
$targetRefShaDisplay = if ($updateTargetRef) {
    if ($targetRefOldSha) {
        "$($targetRefOldSha.Substring(0, 10)) -> $($TargetCommitId.Substring(0, 10))"
    } else {
        "(new) -> $($TargetCommitId.Substring(0, 10))"
    }
} else {
    "$($TargetCommitId.Substring(0, 10)) (unchanged)"
}

Write-Host "Workspace update plan:"
Write-Host "  Upstream ref     : $WorkspaceCommit"
Write-Host "  Workspace head   : $wsStatus"
Write-Host "  Push remote      : $PushRemote"
Write-Host "  Base target ref  : $targetRefDisplay ($targetRefShaDisplay)"
if ($oldTargetCommitId -and $oldTargetCommitId -ne $TargetCommitId) {
    Write-Host ("  Target commit    : {0} -> {1}" -f $oldTargetCommitId.Substring(0, 10), $TargetCommitId.Substring(0, 10))
} else {
    Write-Host ("  Target commit    : {0}" -f $TargetCommitId.Substring(0, 10))
}
Write-Host ""
Write-Host "Applied branches to reset ($($branches.Count)):"

for ($i = 0; $i -lt $branches.Count; $i++) {
    $branch = $branches[$i]
    $newSha = $parents[$i]
    $newShort = $newSha.Substring(0, 10)
    $oldSha = (git --no-pager rev-parse --verify "refs/heads/$branch^{commit}" 2>$null)
    if ($LASTEXITCODE -eq 0 -and $oldSha) {
        $oldSha = $oldSha.Trim()
        $oldShort = $oldSha.Substring(0, 10)
        if ($oldSha -eq $newSha) {
            Write-Host ("  [{0,2}] {1,-36} {2} (unchanged)" -f $i, $branch, $newShort)
        } else {
            Write-Host ("  [{0,2}] {1,-36} {2} -> {3}" -f $i, $branch, $oldShort, $newShort)
        }
    } else {
        Write-Host ("  [{0,2}] {1,-36} (new)      -> {2}" -f $i, $branch, $newShort)
    }
}

if (-not $Yes) {
    Write-Host ""
    $reply = Read-Host "Proceed with resetting workspace and $($branches.Count) applied branches? [y/N]"
    if ($reply -notmatch '^(y|yes)$') {
        Write-Host "Aborted."
        exit 0
    }
}

# 7. Reset local branch refs for each parent of the workspace commit
for ($i = 0; $i -lt $branches.Count; $i++) {
    $branch = $branches[$i]
    $parentSha = $parents[$i]
    git update-ref "refs/heads/$branch" $parentSha

    $remoteRef = "refs/remotes/$PushRemote/$branch"
    git --no-pager show-ref --verify --quiet $remoteRef
    if ($LASTEXITCODE -eq 0) {
        git config "branch.$branch.remote" $PushRemote
        git config "branch.$branch.merge" "refs/heads/$branch"
    }
}

# 8. Configure GitButler project metadata in local git config and sync target git ref
if ($updateTargetRef) {
    git update-ref $TargetRef $TargetCommitId
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to update target ref '$TargetRef' to $TargetCommitId."
    }
}
git config --local gitbutler.project.targetRef $TargetRef
git config --local gitbutler.project.targetCommitId $TargetCommitId
git config --local gitbutler.project.pushRemote $PushRemote
git config --local gitbutler.project.portedMeta true

# 9. Reset refs/heads/gitbutler/workspace and working tree to the upstream workspace commit
git symbolic-ref HEAD refs/heads/gitbutler/workspace
git --no-pager reset --hard $wsSha
if ($LASTEXITCODE -ne 0) {
    throw "Failed to reset gitbutler/workspace to $wsSha."
}

# 10. Write .git/gitbutler/virtual_branches.toml (preserving existing stack UUIDs where possible)
$gitDir = (Resolve-Path (git --no-pager rev-parse --git-dir).Trim()).Path
$gbDir = Join-Path $gitDir "gitbutler"
New-Item -ItemType Directory -Force -Path $gbDir | Out-Null
$tomlPath = Join-Path $gbDir "virtual_branches.toml"

$existingIdsByBranch = @{}
if (Test-Path $tomlPath) {
    $currentSectionId = $null
    foreach ($line in [System.IO.File]::ReadAllLines($tomlPath)) {
        if ($line -match '^\[branches\.([0-9a-fA-F-]{36})\]$') {
            $currentSectionId = $Matches[1].ToLowerInvariant()
        } elseif ($currentSectionId -and $line -match '^name\s*=\s*"([^"]+)"$') {
            $bName = $Matches[1].Trim()
            if ($bName -and -not $existingIdsByBranch.ContainsKey($bName)) {
                $existingIdsByBranch[$bName] = $currentSectionId
            }
        }
    }
}

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

$usedIds = [System.Collections.Generic.HashSet[string]]::new()
for ($i = 0; $i -lt $branches.Count; $i++) {
    $branch = $branches[$i]
    $parentSha = $parents[$i]
    $id = $null
    if ($existingIdsByBranch.ContainsKey($branch)) {
        $candidateId = $existingIdsByBranch[$branch]
        if (-not $usedIds.Contains($candidateId)) {
            $id = $candidateId
        }
    }
    if (-not $id) {
        $id = [guid]::NewGuid().ToString()
    }
    [void]$usedIds.Add($id)

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

[System.IO.File]::WriteAllText($tomlPath, $sb.ToString(), [System.Text.UTF8Encoding]::new($false))

# Remove existing SQLite cache so GitButler imports the updated virtual_branches.toml on next open
Get-ChildItem -Path $gbDir -Filter "but.sqlite*" -ErrorAction SilentlyContinue | Remove-Item -Force

Write-Host "`nDone! Workspace head and $($branches.Count) applied branches have been reset."
