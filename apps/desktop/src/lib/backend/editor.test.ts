import { getEditorOptions, openFileInEditor, VISUAL_STUDIO_EDITOR_ID } from "$lib/backend/editor";
import { describe, expect, test, vi } from "vitest";

describe("getEditorOptions", () => {
	test("includes Visual Studio on Windows", () => {
		expect(getEditorOptions("windows")).toContainEqual({
			schemeIdentifer: VISUAL_STUDIO_EDITOR_ID,
			displayName: "Visual Studio",
		});
	});

	test("does not include Visual Studio on other platforms", () => {
		expect(getEditorOptions("macos")).not.toContainEqual({
			schemeIdentifer: VISUAL_STUDIO_EDITOR_ID,
			displayName: "Visual Studio",
		});
	});
});

describe("openFileInEditor", () => {
	test("uses the validated backend command for Visual Studio", async () => {
		const invoke = vi.fn().mockResolvedValue(undefined);
		const openExternalUrl = vi.fn().mockResolvedValue(undefined);

		await openFileInEditor(
			{ invoke, openExternalUrl },
			{
				editorId: VISUAL_STUDIO_EDITOR_ID,
				projectId: "project-id",
				projectPath: "C:\\repo",
				filePath: "src\\main.rs",
				line: 42,
			},
		);

		expect(invoke).toHaveBeenCalledWith("open_in_program", {
			projectId: "project-id",
			programId: VISUAL_STUDIO_EDITOR_ID,
			path: "src\\main.rs",
			lineNr: 42,
		});
		expect(openExternalUrl).not.toHaveBeenCalled();
	});

	test("preserves URI launching for existing editors", async () => {
		const invoke = vi.fn().mockResolvedValue(undefined);
		const openExternalUrl = vi.fn().mockResolvedValue(undefined);

		await openFileInEditor(
			{ invoke, openExternalUrl },
			{
				editorId: "vscode",
				projectId: "project-id",
				projectPath: "C:\\repo",
				filePath: "src/main.rs",
				line: 42,
			},
		);

		expect(openExternalUrl).toHaveBeenCalledWith("vscode://file/C:/repo/src/main.rs:42");
		expect(invoke).not.toHaveBeenCalled();
	});
});
