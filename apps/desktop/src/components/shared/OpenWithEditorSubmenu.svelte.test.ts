import OpenWithEditorSubmenu from "$components/shared/OpenWithEditorSubmenu.svelte";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterAll, beforeAll, describe, expect, test, vi } from "vitest";

describe("OpenWithEditorSubmenu", () => {
	beforeAll(() => vi.stubGlobal("PointerEvent", MouseEvent));
	afterAll(() => vi.unstubAllGlobals());

	test("offers other supported editors and passes the selected ID", async () => {
		const user = userEvent.setup();
		const onselect = vi.fn();
		render(OpenWithEditorSubmenu, {
			platformName: "windows",
			defaultEditorId: "vscode",
			onselect,
		});

		await user.click(screen.getByText("Open with"));
		expect(await screen.findByText("Visual Studio")).toBeInTheDocument();
		expect(screen.queryByText("VSCode")).not.toBeInTheDocument();

		await user.click(screen.getByText("Visual Studio"));
		expect(onselect).toHaveBeenCalledExactlyOnceWith("visualstudio");
	});

	test("only offers editors supported on the current platform", async () => {
		const user = userEvent.setup();
		render(OpenWithEditorSubmenu, {
			platformName: "macos",
			defaultEditorId: "vscode",
			onselect: vi.fn(),
		});

		await user.click(screen.getByText("Open with"));
		expect(await screen.findByText("Cursor")).toBeInTheDocument();
		expect(screen.queryByText("Visual Studio")).not.toBeInTheDocument();
	});

	test("disables the submenu for deleted files", async () => {
		const user = userEvent.setup();
		const onselect = vi.fn();
		render(OpenWithEditorSubmenu, {
			platformName: "windows",
			defaultEditorId: "vscode",
			disabled: true,
			onselect,
		});

		await user.click(screen.getByText("Open with"));
		expect(screen.queryByText("Visual Studio")).not.toBeInTheDocument();
		expect(onselect).not.toHaveBeenCalled();
	});
});
