import ApplyBranch from "$components/branchesPage/ApplyBranch.svelte";
import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
import { act, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";
import type { ApplyOutcome, ConflictingStack } from "@gitbutler/but-sdk";

const conflict: ConflictingStack = {
	refName: { full: "refs/heads/field-name-cols" },
	shortName: "field-name-cols",
	paths: ["src/GenericEventsTable.cs", "src/OtherTable.cs"],
};

function outcome(conflicts: ConflictingStack[] = []): ApplyOutcome {
	return {
		status: conflicts.length > 0 ? "conflictAborted" : "applied",
		workspaceChanged: true,
		workspaceRefCreated: false,
		appliedBranches: conflicts.length > 0 ? [] : [{ full: "refs/heads/local-build" }],
		conflictingStacks: conflicts,
	};
}

function setup() {
	const applyWithConfirmation = vi.fn().mockResolvedValue(outcome([conflict]));
	const onApplied = vi.fn();
	const view = render(ApplyBranch, {
		props: { projectId: "project", onApplied },
		context: new Map([[STACK_SERVICE._key, { applyWithConfirmation }]]),
	});
	return { ...view, applyWithConfirmation, onApplied, user: userEvent.setup() };
}

describe("applying a conflicting branch", () => {
	test("shows the affected stacks and every file, and cancel does not apply", async () => {
		const { component, applyWithConfirmation, onApplied, user } = setup();
		await act(() => component.apply("refs/heads/local-build"));

		expect(screen.getByRole("heading", { name: "field-name-cols" })).toBeInTheDocument();
		expect(screen.getByText("src/GenericEventsTable.cs")).toBeInTheDocument();
		expect(screen.getByText("src/OtherTable.cs")).toBeInTheDocument();
		expect(applyWithConfirmation).toHaveBeenCalledWith({
			projectId: "project",
			existingBranch: "refs/heads/local-build",
			prNumber: undefined,
			unapplyConflictingStacks: [],
		});
		await user.click(screen.getByRole("button", { name: "Cancel" }));
		await waitFor(() =>
			expect(screen.queryByText("src/GenericEventsTable.cs")).not.toBeInTheDocument(),
		);
		expect(applyWithConfirmation).toHaveBeenCalledOnce();
		expect(onApplied).not.toHaveBeenCalled();
	});

	test("only unapplies the displayed stacks after confirmation", async () => {
		const { component, applyWithConfirmation, onApplied, user } = setup();
		await act(() => component.apply("refs/heads/local-build", 42));
		applyWithConfirmation.mockResolvedValueOnce(outcome());

		await user.click(screen.getByRole("button", { name: "Unapply and continue" }));

		expect(applyWithConfirmation).toHaveBeenLastCalledWith({
			projectId: "project",
			existingBranch: "refs/heads/local-build",
			prNumber: 42,
			unapplyConflictingStacks: ["refs/heads/field-name-cols"],
		});
		await waitFor(() => expect(onApplied).toHaveBeenCalledOnce());
		expect(screen.queryByText("src/GenericEventsTable.cs")).not.toBeInTheDocument();
	});

	test("shows new conflicts instead of navigating when confirmation becomes stale", async () => {
		const { component, applyWithConfirmation, onApplied, user } = setup();
		await act(() => component.apply("refs/heads/local-build"));
		applyWithConfirmation.mockResolvedValueOnce(
			outcome([
				conflict,
				{
					refName: { full: "refs/heads/other" },
					shortName: "other",
					paths: ["src/NewConflict.cs"],
				},
			]),
		);

		await user.click(screen.getByRole("button", { name: "Unapply and continue" }));

		expect(await screen.findByText("src/NewConflict.cs")).toBeInTheDocument();
		expect(onApplied).not.toHaveBeenCalled();
	});

	test("an apply with no stacks to unapply proceeds without a conflict dialog", async () => {
		const { component, applyWithConfirmation, onApplied } = setup();
		applyWithConfirmation.mockResolvedValueOnce(outcome());

		await act(() => component.apply("refs/heads/local-build"));

		expect(onApplied).toHaveBeenCalledOnce();
		expect(screen.queryByText("Branch conflicts with your workspace")).not.toBeInTheDocument();
	});

	test("preserves apply errors instead of replacing them with the unapply dialog", async () => {
		const { component, applyWithConfirmation, onApplied } = setup();
		const error = new Error("Cannot apply branch with conflicting worktree changes");
		applyWithConfirmation.mockRejectedValueOnce(error);

		await expect(component.apply("refs/heads/local-build")).rejects.toBe(error);

		expect(screen.queryByText("Branch conflicts with your workspace")).not.toBeInTheDocument();
		expect(screen.queryByRole("button", { name: "Unapply and continue" })).not.toBeInTheDocument();
		expect(applyWithConfirmation).toHaveBeenCalledOnce();
		expect(onApplied).not.toHaveBeenCalled();
	});
});
