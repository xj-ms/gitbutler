import {
	branchCanUpdateFromRemote,
	branchHasUnpushedCommits,
	branchHasUpstreamCommits,
	branchRequiresForcePush,
	cherryPickTargets,
	toMoveBranchWarning,
	type Stack,
} from "$lib/stacks/stack";
import { TestId } from "@gitbutler/ui";
import { describe, expect, test } from "vitest";
import type { Author, Commit, PushStatus, Segment, UpstreamCommit } from "@gitbutler/but-sdk";

function stackWithBranches(...branchNames: (string | null)[]): Stack {
	return {
		id: "stack-1",
		base: null,
		segments: branchNames.map((name) => ({
			refName: name === null ? null : { displayName: name, fullNameBytes: [] },
		})),
	} as Stack;
}

describe("toMoveBranchWarning", () => {
	test("returns undefined when tearing off leaves all stacks applied", () => {
		const beforeAppliedStackCount = 3;
		const afterAppliedStackCount = 4;
		const unappliedStackCount = Math.max(0, beforeAppliedStackCount + 1 - afterAppliedStackCount);

		expect(toMoveBranchWarning(unappliedStackCount)).toBeUndefined();
	});

	test("renders a singular warning message when one stack is unapplied", () => {
		const beforeAppliedStackCount = 3;
		const afterAppliedStackCount = 3;
		const unappliedStackCount = Math.max(0, beforeAppliedStackCount + 1 - afterAppliedStackCount);
		const warning = toMoveBranchWarning(unappliedStackCount);

		if (!warning || warning.type !== "warning") {
			throw new Error("Expected a warning drop result");
		}

		expect(warning).toMatchObject({
			type: "warning",
			title: "Heads up: We had to unapply some stacks to move this branch",
			testId: TestId.StacksUnappliedToast,
		});
		expect(warning.message).toContain("1 stack");
	});

	test("renders a plural warning message when multiple stacks are unapplied", () => {
		const beforeAppliedStackCount = 3;
		const afterAppliedStackCount = 2;
		const unappliedStackCount = Math.max(0, beforeAppliedStackCount + 1 - afterAppliedStackCount);
		const warning = toMoveBranchWarning(unappliedStackCount);

		if (!warning || warning.type !== "warning") {
			throw new Error("Expected a warning drop result");
		}

		expect(warning.message).toContain("2 stacks");
	});
});

describe("cherryPickTargets", () => {
	test("identifies each stack by its top branch and counts its branches", () => {
		const targets = cherryPickTargets([
			stackWithBranches("feature", "base"),
			stackWithBranches("solo"),
		]);

		expect(targets).toEqual([
			{ branchName: "feature", branchCount: 2 },
			{ branchName: "solo", branchCount: 1 },
		]);
	});

	test("skips stacks whose top branch lost its name", () => {
		const targets = cherryPickTargets([
			stackWithBranches(null, "base"),
			stackWithBranches("feature"),
		]);

		expect(targets).toEqual([{ branchName: "feature", branchCount: 1 }]);
	});
});

const encoder = new TextEncoder();

function bytes(value: string): number[] {
	return [...encoder.encode(value)];
}

const author: Author = {
	name: "Ada",
	email: "ada@example.com",
	gravatarUrl: "",
};

// A local commit that shares its change id with a remote commit but has a
// different commit id, i.e. it was rewritten locally and needs a force push.
const rewrittenCommit: Commit = {
	id: "1111111111111111111111111111111111111111",
	parentIds: ["0000000000000000000000000000000000000000"],
	message: "Rewritten commit",
	hasConflicts: false,
	state: { type: "LocalAndRemote", subject: "2222222222222222222222222222222222222222" },
	authoredAt: 1000,
	committedAt: 1000,
	author,
	changeId: "I111",
	gerritReviewUrl: null,
};

// A commit that only exists on the remote (something to integrate).
const upstreamCommit: UpstreamCommit = {
	id: "3333333333333333333333333333333333333333",
	message: "Upstream only commit",
	authoredAt: 2000,
	committedAt: 2000,
	author,
	changeId: "I333",
};

function segment(overrides: Partial<Segment> = {}): Segment {
	return {
		refName: {
			fullNameBytes: bytes("refs/heads/feature/top"),
			displayName: "feature/top",
		},
		remoteTrackingRefName: {
			fullNameBytes: bytes("refs/remotes/origin/feature/top"),
			displayName: "feature/top",
			remoteName: "origin",
		},
		commits: [rewrittenCommit],
		commitsOnRemote: [],
		commitsOutside: null,
		metadata: null,
		isEntrypoint: true,
		pushStatus: "unpushedCommitsRequiringForce" as PushStatus,
		base: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		...overrides,
	};
}

describe("branch action gating", () => {
	test("force push and update are gated independently in true both-sided divergence", () => {
		// The branch was rewritten locally (force push required) AND the remote has
		// commits the local branch lacks (update available). Both actions must be
		// offered together.
		const diverged = segment({ commitsOnRemote: [upstreamCommit] });

		expect(branchRequiresForcePush(diverged)).toBe(true);
		expect(branchHasUpstreamCommits(diverged)).toBe(true);
		expect(branchCanUpdateFromRemote(diverged)).toBe(true);
	});

	test("local rewrite (e.g. amended upstream) offers both force push and update", () => {
		// The remote holds a rewritten/amended version of a local commit (same
		// change-id, different SHA). Such commits are matched to their local
		// counterpart and excluded from commitsOnRemote, yet the branches have
		// diverged from the merge-base and the integration flow can reconcile them,
		// so the update action must still be offered alongside force push.
		const rewriteOnly = segment({ commitsOnRemote: [] });

		expect(branchRequiresForcePush(rewriteOnly)).toBe(true);
		expect(branchHasUpstreamCommits(rewriteOnly)).toBe(false);
		expect(branchCanUpdateFromRemote(rewriteOnly)).toBe(true);
	});

	test("upstream-only divergence offers update but not force push", () => {
		// The remote is ahead, local has nothing needing a force push.
		const upstreamOnly = segment({
			commits: [],
			commitsOnRemote: [upstreamCommit],
			pushStatus: "nothingToPush" as PushStatus,
		});

		expect(branchRequiresForcePush(upstreamOnly)).toBe(false);
		expect(branchHasUpstreamCommits(upstreamOnly)).toBe(true);
		expect(branchHasUnpushedCommits(upstreamOnly)).toBe(false);
		expect(branchCanUpdateFromRemote(upstreamOnly)).toBe(true);
	});

	test("in-sync branch offers neither force push nor update", () => {
		// No divergence at all: nothing to force push and nothing to integrate.
		const inSync = segment({
			commits: [],
			commitsOnRemote: [],
			pushStatus: "nothingToPush" as PushStatus,
		});

		expect(branchRequiresForcePush(inSync)).toBe(false);
		expect(branchHasUpstreamCommits(inSync)).toBe(false);
		expect(branchCanUpdateFromRemote(inSync)).toBe(false);
	});
});
