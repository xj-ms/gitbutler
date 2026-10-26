import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const repoRoot = process.cwd();
const gitDir = resolve(repoRoot, ".git");

if (!existsSync(gitDir)) {
	process.exit(0);
}

/**
 * @param {string[]} args
 * @param {{ cwd?: string; allowFailure?: boolean }} [options]
 */
function runGit(args, options = {}) {
	try {
		return execFileSync("git", args, {
			cwd: options.cwd ?? repoRoot,
			encoding: "utf8",
			stdio: ["ignore", "pipe", "pipe"],
		}).trim();
	} catch (error) {
		if (options.allowFailure) {
			return "";
		}
		throw error;
	}
}

// 1. Point Git hooks at the tracked .githooks directory.
runGit(["config", "core.hooksPath", ".githooks"], { allowFailure: true });

// 2. Ensure vendor/gitoxide is configured for a sparse checkout of /gix-merge/ only.
const submodulePath = "vendor/gitoxide";
const submoduleUrl = "https://github.com/xj-ms/gitoxide.git";
const sparsePattern = "/gix-merge/\n";
const moduleGitDir = resolve(repoRoot, ".git/modules", submodulePath);
const submoduleWorktree = resolve(repoRoot, submodulePath);
const gixMergeManifest = resolve(submoduleWorktree, "gix-merge/Cargo.toml");

if (!existsSync(resolve(moduleGitDir, "config"))) {
	mkdirSync(dirname(moduleGitDir), { recursive: true });
	runGit([
		"clone",
		"--no-checkout",
		"--filter=blob:none",
		"--separate-git-dir",
		moduleGitDir,
		submoduleUrl,
		submoduleWorktree,
	]);
}

runGit(["config", `submodule.${submodulePath}.url`, submoduleUrl], { allowFailure: true });
runGit(["config", "-f", resolve(moduleGitDir, "config"), "core.sparseCheckout", "true"]);
runGit(["config", "-f", resolve(moduleGitDir, "config"), "core.sparseCheckoutCone", "false"]);

const sparseCheckoutFile = resolve(moduleGitDir, "info/sparse-checkout");
mkdirSync(dirname(sparseCheckoutFile), { recursive: true });
const currentSparse = existsSync(sparseCheckoutFile)
	? readFileSync(sparseCheckoutFile, "utf8")
	: "";

if (currentSparse !== sparsePattern) {
	writeFileSync(sparseCheckoutFile, sparsePattern, "utf8");
	if (existsSync(gixMergeManifest)) {
		runGit(["read-tree", "-mu", "HEAD"], { cwd: submoduleWorktree, allowFailure: true });
	}
}

// 3. Materialize or update the submodule worktree if needed.
const gitlinkLine = runGit(["ls-tree", "HEAD", submodulePath], { allowFailure: true });
const expectedSha = gitlinkLine.split(/\s+/)[2];

if (expectedSha) {
	const currentSha = existsSync(submoduleWorktree)
		? runGit(["rev-parse", "HEAD"], { cwd: submoduleWorktree, allowFailure: true })
		: "";
	if (currentSha !== expectedSha || !existsSync(gixMergeManifest)) {
		runGit(["submodule", "update", "--init", "--filter=blob:none", submodulePath], {
			allowFailure: true,
		});
	}
} else if (!existsSync(gixMergeManifest)) {
	runGit(["checkout", "gitbutler-0.23.3"], { cwd: submoduleWorktree, allowFailure: true });
}
