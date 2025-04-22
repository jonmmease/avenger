package tree_sitter_avenger_test

import (
	"testing"

	tree_sitter "github.com/tree-sitter/go-tree-sitter"
	tree_sitter_avenger "github.com/jonmmease/avenger/bindings/go"
)

func TestCanLoadGrammar(t *testing.T) {
	language := tree_sitter.NewLanguage(tree_sitter_avenger.Language())
	if language == nil {
		t.Errorf("Error loading Avenger grammar")
	}
}
