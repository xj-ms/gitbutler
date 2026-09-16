import ProjectSelectItem from "$components/shared/ProjectSelectItem.svelte";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";

describe("ProjectSelectItem", () => {
	test("does not show a remove action when removal is disabled", () => {
		render(ProjectSelectItem, {
			projectId: "current-project",
			projectTitle: "Current project",
			removable: false,
			onRemove: vi.fn(),
		});

		expect(
			screen.queryByRole("button", { name: "Remove Current project from GitButler" }),
		).not.toBeInTheDocument();
	});

	test("invokes its remove action", async () => {
		const user = userEvent.setup();
		const onRemove = vi.fn();
		render(ProjectSelectItem, {
			projectId: "other-project",
			projectTitle: "Other project",
			removable: true,
			onRemove,
		});

		const removeButton = screen.getByRole("button", {
			name: "Remove Other project from GitButler",
		});
		await user.click(removeButton);

		expect(onRemove).toHaveBeenCalledOnce();
	});

	test("supports keyboard activation", async () => {
		const user = userEvent.setup();
		const onRemove = vi.fn();
		render(ProjectSelectItem, {
			projectId: "other-project",
			projectTitle: "Other project",
			removable: true,
			onRemove,
		});

		const removeButton = screen.getByRole("button", {
			name: "Remove Other project from GitButler",
		});
		removeButton.focus();
		await user.keyboard("{Enter}");

		expect(onRemove).toHaveBeenCalledOnce();
	});
});
