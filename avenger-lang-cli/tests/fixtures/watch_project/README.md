# Watch integration fixture

This project exercises the stock `avenger watch` host with one chart root, a
relative mark definition, a catalog/schema declaration, local CSV data, and a
typed parameter. Its direct `float64` canvas-size parameters also exercise the
stock CLI's virtual-canvas resize binding. Tests copy the complete directory
before mutating sources so the checked-in fixture remains stable.
