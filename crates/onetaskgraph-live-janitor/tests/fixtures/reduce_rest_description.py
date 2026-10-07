"""Reduce GitHub's published REST description to the operations the janitor sends.

Usage: python3 -I reduce_rest_description.py <api.github.com.json> <commit> <date> > rest-operations.json

The input is descriptions/api.github.com/api.github.com.json of
https://github.com/github/rest-api-description at <commit>. Re-run it against a newer
commit to re-observe GitHub's contract; the offline journeys then hold the janitor and
its loopback stand-in to whatever the description says.
"""
import json
import sys

# (method, path, the response fields descended into, as dotted paths)
OPERATIONS = [
    ('get', '/rate_limit', ['resources.core', 'resources.graphql']),
    ('get', '/repos/{owner}/{repo}/issues', ['[]']),
    ('get', '/repos/{owner}/{repo}/labels', ['[]']),
    ('delete', '/repos/{owner}/{repo}/labels/{name}', []),
    ('get', '/repos/{owner}/{repo}/actions/workflows/{workflow_id}/runs', ['workflow_runs.[]']),
    ('get', '/repos/{owner}/{repo}/actions/runs/{run_id}', []),
]


def main(description_path, commit, date):
    description = json.load(open(description_path))

    def deref(node):
        while isinstance(node, dict) and '$ref' in node:
            target = description
            for key in node['$ref'][2:].split('/'):
                target = target[key]
            node = target
        return node

    def properties(schema):
        schema = deref(schema)
        result = {}
        for combinator in ('allOf', 'anyOf', 'oneOf'):
            for member in schema.get(combinator, []):
                result.update(properties(member))
        result.update(schema.get('properties', {}))
        return result

    def shape(schema, descents, prefix=''):
        schema = deref(schema)
        if schema.get('type') == 'array':
            here = prefix + '[]'
            inner = shape(schema['items'], descents, here + '.') if any(d == here or d.startswith(here + '.') for d in descents) else None
            return {'[]': inner}
        result = {}
        for name, value in sorted(properties(schema).items()):
            here = prefix + name
            descend = any(d == here or d.startswith(here + '.') for d in descents)
            result[name] = shape(value, descents, here + '.') if descend else None
        return result

    operations = []
    for method, path, descents in OPERATIONS:
        operation = description['paths'][path][method]
        query = {}
        for parameter in map(deref, operation.get('parameters', [])):
            if parameter['in'] == 'query':
                query[parameter['name']] = deref(parameter.get('schema', {})).get('enum')
        success = next(code for code in sorted(operation['responses']) if code.startswith('2'))
        body = deref(operation['responses'][success]).get('content', {}).get('application/json', {}).get('schema')
        operations.append({
            'operation_id': operation['operationId'],
            'method': method.upper(),
            'path': path,
            'query': query,
            'status': int(success),
            'response': shape(body, descents) if body else None,
        })
    json.dump({
        '_': [
            'The REST operations the live janitor sends, reduced from GitHub\'s published',
            'OpenAPI description by reduce_rest_description.py beside this file:',
            f'github/rest-api-description@{commit}, descriptions/api.github.com/api.github.com.json,',
            f'read {date}. Documentation-derived; no response here was captured live.',
            'A response field mapped to null is present and not descended into.',
            'tests/journey.py holds every request the janitor sends and every field its',
            'loopback stand-in serves to this pin, so neither can drift from GitHub\'s contract.',
        ],
        'operations': operations,
    }, sys.stdout, indent=2)
    sys.stdout.write('\n')


if __name__ == '__main__':
    main(*sys.argv[1:])
