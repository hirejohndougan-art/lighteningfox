// Counts how many times this module runs; imported and also added as <script type=module>.
window.moduleRuns = (window.moduleRuns || 0) + 1;
export const runs = () => window.moduleRuns;
