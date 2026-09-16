import Component from "$components/test/project-select-remove/Component.svelte";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeAll, expect, test, vi } from "vitest";

beforeAll(() => {
	vi.stubGlobal(
		"ResizeObserver",
		class {
			observe() {}
			unobserve() {}
			disconnect() {}
		},
	);
});

test("removal closes the selector without selecting the project", async () => {
	const user = userEvent.setup();
	const onRemove = vi.fn();
	const onSelect = vi.fn();
	render(Component, { onRemove, onSelect });

	await user.click(screen.getByRole("button", { name: "Open projects" }));
	await user.click(screen.getByRole("button", { name: "Remove Other project from GitButler" }));

	expect(onRemove).toHaveBeenCalledOnce();
	expect(onSelect).not.toHaveBeenCalled();
	expect(
		screen.queryByRole("button", { name: "Remove Other project from GitButler" }),
	).not.toBeInTheDocument();
});
