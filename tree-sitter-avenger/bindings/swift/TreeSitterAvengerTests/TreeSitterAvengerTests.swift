import XCTest
import SwiftTreeSitter
import TreeSitterAvenger

final class TreeSitterAvengerTests: XCTestCase {
    func testCanLoadGrammar() throws {
        let parser = Parser()
        let language = Language(language: tree_sitter_avenger())
        XCTAssertNoThrow(try parser.setLanguage(language),
                         "Error loading Avenger grammar")
    }
}
