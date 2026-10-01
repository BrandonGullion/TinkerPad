// A mistake with a line number, for [output].error_regex.
const nothing: unknown = undefined;

(nothing as { go: () => void }).go();
