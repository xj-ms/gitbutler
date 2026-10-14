const highlightCounts = new WeakMap<Element, number>();

export function highlightDependencyCommitRows(commitIds: string[], filePath?: string): () => void {
	const highlightedRows = new Set<Element>();
	for (const commitId of commitIds) {
		const commitRows = document.querySelectorAll(`[data-commit-id="${commitId}"]`);
		commitRows.forEach((row) => {
			highlightedRows.add(row);
			if (filePath !== undefined) {
				const fileRows = row.closest(".commit-row-wrapper")?.querySelectorAll("[data-file-path]");
				fileRows?.forEach((fileRow) => {
					if (fileRow.getAttribute("data-file-path") === filePath) {
						highlightedRows.add(fileRow);
					}
				});
			}
		});
	}

	highlightedRows.forEach((row) => {
		highlightCounts.set(row, (highlightCounts.get(row) ?? 0) + 1);
		row.classList.add("dependency-highlighted");
	});

	let active = true;
	return () => {
		if (!active) return;
		active = false;
		highlightedRows.forEach((row) => {
			const remaining = (highlightCounts.get(row) ?? 1) - 1;
			if (remaining === 0) {
				highlightCounts.delete(row);
				row.classList.remove("dependency-highlighted");
			} else {
				highlightCounts.set(row, remaining);
			}
		});
	};
}
