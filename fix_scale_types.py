#!/usr/bin/env python3
import os
import re
import sys

# Mapping from string scale types to their imports and types
SCALE_MAPPING = {
    "linear": ("use avenger_scales::scales::linear::LinearScale;", "LinearScale"),
    "log": ("use avenger_scales::scales::log::LogScale;", "LogScale"),
    "band": ("use avenger_scales::scales::band::BandScale;", "BandScale"),
    "ordinal": ("use avenger_scales::scales::ordinal::OrdinalScale;", "OrdinalScale"),
    "point": ("use avenger_scales::scales::point::PointScale;", "PointScale"),
    "pow": ("use avenger_scales::scales::pow::PowScale;", "PowScale"),
    "sqrt": ("use avenger_scales::scales::pow::PowScale;", "PowScale"),  # sqrt uses PowScale
    "threshold": ("use avenger_scales::scales::threshold::ThresholdScale;", "ThresholdScale"),
    "time": ("use avenger_scales::scales::time::TimeScale;", "TimeScale"),
}

def fix_file(filepath):
    with open(filepath, 'r') as f:
        content = f.read()
    
    # Find all scale_type calls
    pattern = r'\.scale_type\("([^"]+)"\)'
    matches = re.findall(pattern, content)
    
    if not matches:
        return False
    
    # Track which imports we need
    needed_imports = set()
    scale_types_used = set()
    
    for match in matches:
        if match in SCALE_MAPPING:
            import_stmt, scale_type = SCALE_MAPPING[match]
            needed_imports.add(import_stmt)
            scale_types_used.add((match, scale_type))
        else:
            print(f"Warning: Unknown scale type '{match}' in {filepath}")
    
    # Replace scale_type calls
    modified = content
    for string_type, scale_type in scale_types_used:
        # Special case for sqrt - needs to also set exponent
        if string_type == "sqrt":
            # Replace .scale_type("sqrt") with .scale_type(PowScale).option("exponent", 0.5)
            modified = modified.replace(
                f'.scale_type("{string_type}")',
                f'.scale_type({scale_type}).option("exponent", 0.5)'
            )
        else:
            modified = modified.replace(f'.scale_type("{string_type}")', f'.scale_type({scale_type})')
    
    # Add imports after existing use statements (find last use statement)
    use_lines = []
    other_lines = []
    in_use_block = False
    
    for line in modified.split('\n'):
        if line.startswith('use '):
            use_lines.append(line)
            in_use_block = True
        elif in_use_block and line and not line.startswith('//'):
            # End of use block
            in_use_block = False
            other_lines.append(line)
        else:
            other_lines.append(line)
    
    # Add our imports to the use block
    for import_stmt in needed_imports:
        if import_stmt not in '\n'.join(use_lines):
            use_lines.append(import_stmt)
    
    # Reconstruct the file
    if use_lines:
        result = '\n'.join(use_lines) + '\n' + '\n'.join(other_lines)
    else:
        # No existing use statements, add at the top after any module docs
        lines = modified.split('\n')
        insert_idx = 0
        for i, line in enumerate(lines):
            if not line.startswith('//') and line.strip():
                insert_idx = i
                break
        
        lines[insert_idx:insert_idx] = list(needed_imports) + ['']
        result = '\n'.join(lines)
    
    with open(filepath, 'w') as f:
        f.write(result)
    
    return True

def main():
    test_dir = "/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/tests"
    
    # Find all test files
    test_files = []
    for root, dirs, files in os.walk(test_dir):
        for file in files:
            if file.endswith('.rs'):
                test_files.append(os.path.join(root, file))
    
    fixed_count = 0
    for filepath in test_files:
        if fix_file(filepath):
            print(f"Fixed: {filepath}")
            fixed_count += 1
    
    print(f"\nFixed {fixed_count} files")

if __name__ == "__main__":
    main()