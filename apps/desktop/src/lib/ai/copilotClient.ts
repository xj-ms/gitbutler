import {
	SHORT_DEFAULT_BRANCH_TEMPLATE,
	SHORT_DEFAULT_COMMIT_TEMPLATE,
	SHORT_DEFAULT_PR_TEMPLATE,
} from "$lib/ai/prompts";
import type { AIClient, AIEvalOptions, Prompt } from "$lib/ai/types";
import type { IBackend } from "$lib/backend";

export const DEFAULT_COPILOT_MODEL_NAME = "default";

/**
 * CopilotClient implements AIClient backed by GitHub Copilot SDK (`copilot-sdk`)
 * in the Rust backend, authenticating solely via the user's existing Copilot CLI login.
 */
export class CopilotClient implements AIClient {
	defaultCommitTemplate = SHORT_DEFAULT_COMMIT_TEMPLATE;
	defaultBranchTemplate = SHORT_DEFAULT_BRANCH_TEMPLATE;
	defaultPRTemplate = SHORT_DEFAULT_PR_TEMPLATE;

	constructor(
		private backend: IBackend,
		private modelName: string = DEFAULT_COPILOT_MODEL_NAME,
	) {}

	async evaluate(prompt: Prompt, options?: AIEvalOptions): Promise<string> {
		if (!prompt || !Array.isArray(prompt) || prompt.length === 0) {
			throw new Error("Invalid prompt: must be a non-empty array");
		}

		const messages = prompt.map((msg) => ({
			role: msg.role.toLowerCase(),
			content: msg.content,
		}));

		const response = await this.backend.invoke<string>("ai_copilot_evaluate", {
			messages,
			model: this.modelName,
		});

		if (response && options?.onToken) {
			options.onToken(response);
		}

		return response;
	}
}
