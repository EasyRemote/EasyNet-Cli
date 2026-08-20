package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

type inventory struct {
	Symbols []string          `json:"symbols"`
	Members []string          `json:"members"`
	Shapes  map[string]string `json:"shapes"`
}

func main() {
	directory := flag.String("dir", "", "directory containing one Go package")
	file := flag.String("file", "", "single Go source file")
	flag.Parse()
	if (*directory == "") == (*file == "") {
		fail("exactly one of -dir or -file is required")
	}
	files, err := parseFiles(*directory, *file)
	if err != nil {
		fail(err.Error())
	}
	symbols := map[string]struct{}{}
	members := map[string]struct{}{}
	shapes := map[string]string{}
	for _, parsed := range files {
		collect(parsed, symbols, members, shapes)
	}
	result := inventory{Symbols: sortedKeys(symbols), Members: sortedKeys(members), Shapes: shapes}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		fail(err.Error())
	}
}

func parseFiles(directory, file string) ([]*ast.File, error) {
	set := token.NewFileSet()
	if file != "" {
		parsed, err := parser.ParseFile(set, file, nil, 0)
		if err != nil {
			return nil, err
		}
		return []*ast.File{parsed}, nil
	}
	packages, err := parser.ParseDir(set, directory, func(info os.FileInfo) bool {
		return !strings.HasSuffix(info.Name(), "_test.go")
	}, 0)
	if err != nil {
		return nil, err
	}
	if len(packages) != 1 {
		return nil, fmt.Errorf("%s must contain exactly one Go package", filepath.Clean(directory))
	}
	var files []*ast.File
	for _, pkg := range packages {
		for _, parsed := range pkg.Files {
			files = append(files, parsed)
		}
	}
	return files, nil
}

func collect(file *ast.File, symbols, members map[string]struct{}, shapes map[string]string) {
	for _, declaration := range file.Decls {
		switch value := declaration.(type) {
		case *ast.FuncDecl:
			if !value.Name.IsExported() {
				continue
			}
			if value.Recv == nil {
				symbols[value.Name.Name] = struct{}{}
				recordShape(shapes, value.Name.Name, functionShape(value.Type))
				continue
			}
			if receiver := receiverName(value.Recv.List[0].Type); receiver != "" && ast.IsExported(receiver) {
				name := receiver + "." + value.Name.Name
				members[name] = struct{}{}
				recordShape(shapes, name, functionShape(value.Type))
			}
		case *ast.GenDecl:
			for _, spec := range value.Specs {
				switch item := spec.(type) {
				case *ast.TypeSpec:
					if item.Name.IsExported() {
						symbols[item.Name.Name] = struct{}{}
						recordShape(shapes, item.Name.Name, publicTypeShape(item.Type))
						collectTypeMembers(item.Name.Name, item.Type, members, shapes)
					}
				case *ast.ValueSpec:
					for index, name := range item.Names {
						if name.IsExported() {
							symbols[name.Name] = struct{}{}
							shape := nodeShape(item.Type)
							if index < len(item.Values) {
								shape += "=" + nodeShape(item.Values[index])
							} else if len(item.Values) == 1 {
								shape += "=" + nodeShape(item.Values[0])
							}
							recordShape(shapes, name.Name, shape)
						}
					}
				}
			}
		}
	}
}

func collectTypeMembers(typeName string, expression ast.Expr, members map[string]struct{}, shapes map[string]string) {
	var fields *ast.FieldList
	switch value := expression.(type) {
	case *ast.StructType:
		fields = value.Fields
	case *ast.InterfaceType:
		fields = value.Methods
	default:
		return
	}
	for _, field := range fields.List {
		for _, name := range field.Names {
			if name.IsExported() {
				member := typeName + "." + name.Name
				members[member] = struct{}{}
				recordShape(shapes, member, publicMemberShape(field.Type))
			}
		}
		if len(field.Names) == 0 {
			if embedded := receiverName(field.Type); ast.IsExported(embedded) {
				member := typeName + "." + embedded
				members[member] = struct{}{}
				recordShape(shapes, member, publicMemberShape(field.Type))
			}
		}
	}
}

func recordShape(shapes map[string]string, name, shape string) {
	if previous, ok := shapes[name]; ok && previous != shape {
		fail(fmt.Sprintf("conflicting public shape for %s: %s != %s", name, previous, shape))
	}
	shapes[name] = shape
}

func publicMemberShape(node ast.Node) string {
	if function, ok := node.(*ast.FuncType); ok {
		return functionShape(function)
	}
	return nodeShape(node)
}

func publicTypeShape(expression ast.Expr) string {
	if structure, ok := expression.(*ast.StructType); ok {
		return nodeShape(scrubStructPublicFields(structure))
	}
	return nodeShape(expression)
}

func scrubStructPublicFields(structure *ast.StructType) *ast.StructType {
	clone := *structure
	clone.Fields = &ast.FieldList{List: []*ast.Field{}}
	if structure.Fields == nil {
		return &clone
	}
	for _, field := range structure.Fields.List {
		publicNames := make([]*ast.Ident, 0, len(field.Names))
		for _, name := range field.Names {
			if name.IsExported() {
				publicNames = append(publicNames, name)
			}
		}
		if len(field.Names) > 0 && len(publicNames) == 0 {
			continue
		}
		if len(field.Names) == 0 && !ast.IsExported(receiverName(field.Type)) {
			continue
		}
		fieldClone := *field
		fieldClone.Names = publicNames
		clone.Fields.List = append(clone.Fields.List, &fieldClone)
	}
	return &clone
}

func functionShape(function *ast.FuncType) string {
	if function == nil {
		return "inferred"
	}
	clone := *function
	clone.Params = scrubFieldListNames(function.Params)
	clone.Results = scrubFieldListNames(function.Results)
	return nodeShape(&clone)
}

func scrubFieldListNames(fields *ast.FieldList) *ast.FieldList {
	if fields == nil {
		return nil
	}
	clone := *fields
	clone.List = make([]*ast.Field, 0, len(fields.List))
	for _, field := range fields.List {
		fieldClone := *field
		fieldClone.Names = nil
		clone.List = append(clone.List, &fieldClone)
	}
	return &clone
}

func nodeShape(node ast.Node) string {
	if node == nil {
		return "inferred"
	}
	var output bytes.Buffer
	if err := format.Node(&output, token.NewFileSet(), node); err != nil {
		fail(err.Error())
	}
	return output.String()
}

func receiverName(expression ast.Expr) string {
	switch value := expression.(type) {
	case *ast.Ident:
		return value.Name
	case *ast.StarExpr:
		return receiverName(value.X)
	case *ast.IndexExpr:
		return receiverName(value.X)
	case *ast.IndexListExpr:
		return receiverName(value.X)
	default:
		return ""
	}
}

func sortedKeys(values map[string]struct{}) []string {
	result := make([]string, 0, len(values))
	for value := range values {
		result = append(result, value)
	}
	sort.Strings(result)
	return result
}

func fail(message string) {
	fmt.Fprintln(os.Stderr, "sdk-api-inventory:", message)
	os.Exit(1)
}
