import { getEditorUri } from "$lib/backend/url";
import { showToast } from "$lib/notifications/toasts";
import { vscodePath } from "$lib/project/project";
import type { IBackend } from "$lib/backend";
import type { CodeEditorSettings } from "$lib/state/uiState.svelte";

export const VISUAL_STUDIO_EDITOR_ID = "visualstudio";

const EDITOR_OPTIONS: CodeEditorSettings[] = [
	{ schemeIdentifer: "vscodium", displayName: "VSCodium" },
	{ schemeIdentifer: "vscode", displayName: "VSCode" },
	{ schemeIdentifer: "vscode-insiders", displayName: "VSCode Insiders" },
	{ schemeIdentifer: "windsurf", displayName: "Windsurf" },
	{ schemeIdentifer: "zed", displayName: "Zed" },
	{ schemeIdentifer: "cursor", displayName: "Cursor" },
	{ schemeIdentifer: "trae", displayName: "Trae" },
	{ schemeIdentifer: "antigravity-ide", displayName: "Antigravity IDE" },
];

export function getEditorOptions(platformName: string): CodeEditorSettings[] {
	if (platformName !== "windows") return EDITOR_OPTIONS;

	return [
		...EDITOR_OPTIONS,
		{ schemeIdentifer: VISUAL_STUDIO_EDITOR_ID, displayName: "Visual Studio" },
	];
}

type EditorBackend = Pick<IBackend, "invoke" | "openExternalUrl">;

type OpenFileInEditorParams = {
	editorId: string;
	projectId: string;
	projectPath: string;
	filePath: string;
	line?: number;
};

export async function openFileInEditor(
	backend: EditorBackend,
	params: OpenFileInEditorParams,
): Promise<boolean> {
	try {
		if (params.editorId === VISUAL_STUDIO_EDITOR_ID) {
			await backend.invoke("open_in_program", {
				projectId: params.projectId,
				programId: params.editorId,
				path: params.filePath,
				lineNr: params.line,
			});
		} else {
			await backend.openExternalUrl(
				getEditorUri({
					schemeId: params.editorId,
					path: [vscodePath(params.projectPath), params.filePath],
					line: params.line,
				}),
			);
		}
		return true;
	} catch (error: unknown) {
		showToast({
			title: "Failed to open in editor",
			message: error instanceof Error ? error.message : String(error),
			error,
			style: "danger",
		});
		return false;
	}
}
