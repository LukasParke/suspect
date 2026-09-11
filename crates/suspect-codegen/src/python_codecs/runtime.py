"""Conversion from checked native model descriptors with shared per-call budgets."""
from __future__ import annotations
import functools
import json
import pathlib
from typing import Any, Generic, TypeVar, cast, TYPE_CHECKING, NoReturn
if TYPE_CHECKING or __package__:
    from . import json_runtime as J, models as M, validation as V
else:
    import json_runtime as J
    import models as M
    import validation as V

T = TypeVar('T')

class CodecError(Exception):
    def __init__(self, kind: str, source: str, path: str, message: str) -> None:
        super().__init__(f'{kind} at {source} {path}: {message}')
        self.kind, self.source, self.path, self.message = kind, source, path, message

@functools.lru_cache(maxsize=1)
def _plan() -> dict[str, Any]:
    return cast(dict[str, Any], json.loads(pathlib.Path(__file__).with_name('codec-plan.json').read_text(encoding='utf-8')))

class _Context:
    def __init__(self, name: str) -> None:
        self.plan = _plan()
        self.validation = V.ValidationSession()
        self.remaining: int = self.plan['maxSteps']
        self.active: set[int] = set()
        self.location: dict[str, Any] = self.plan['models'][name]['source']
        self.source: str = self.location['document'] + '#' + self.location['pointer']
        self.json_limits = J.JsonLimits(**self.plan['json'])
    def step(self, path: str, depth: int, count: int = 1) -> None:
        self.remaining -= count
        if self.remaining < 0 or depth > self.plan['maxDepth']:
            raise CodecError('resource', self.source, path, 'conversion budget exhausted')
    def fail(self, path: str, message: str) -> NoReturn:
        raise CodecError('conversion', self.source, path, message)
    def check(self, index: int, value: Any) -> None:
        try:
            self.validation.check(index, value)
        except V.ValidationError as error:
            raise CodecError('resource' if error.kind == 'evaluation_failure' else 'invalid',error.source,error.instance_path,error.message) from error
    def equal(self, left: Any, right: Any, path: str) -> bool:
        try:
            return self.validation.equal(left,right,self.location,path)
        except V.ValidationError as error:
            raise CodecError('resource' if error.kind == 'evaluation_failure' else 'invalid',error.source,error.instance_path,error.message) from error
    def text(self, value: str, path: str, depth: int) -> None:
        self.step(path,depth,len(value))
        if not value.isascii():
            try: extra = len(value.encode('utf-8','strict')) - len(value)
            except UnicodeEncodeError: self.fail(path,'string contains a lone surrogate')
            self.step(path,depth,extra)
    def charge_json(self, value: Any, path: str, depth: int) -> None:
        self.step(path,depth)
        kind = type(value)
        if kind is str: self.text(value,path,depth)
        elif kind is J.JsonNumber: self.text(value.token,path,depth)
        elif kind is int: self.step(path,depth,max(1,value.bit_length() // 3 + 1))
        elif kind is list or kind is dict:
            marker = id(value)
            if marker in self.active: self.fail(path,'cyclic JSON value')
            self.active.add(marker)
            try:
                if kind is list:
                    for index,item in enumerate(value): self.charge_json(item,_child(path,str(index)),depth + 1)
                else:
                    for key,item in value.items():
                        if type(key) is not str: self.fail(path,'object key is not a string')
                        self.text(key,path,depth)
                        self.charge_json(item,_child(path,key),depth + 1)
            finally: self.active.remove(marker)
        elif value is not None and kind is not bool: self.fail(path,'value is outside the exact JSON representation')
    def clone_json(self, value: Any, path: str = '', depth: int = 0) -> J.JsonValue:
        self.charge_json(value,path,depth)
        try: return J.parse_json(J.stringify_json(value,self.json_limits),self.json_limits)
        except J.JsonError as error:
            raise CodecError('resource' if error.kind == J.RESOURCE_LIMIT else 'conversion',self.source,path,error.message) from error
    def expression(self, ty: dict[str, Any], value: Any, path: str, depth: int, encode: bool) -> Any:
        self.step(path,depth)
        kind = ty['kind']
        if kind == 'named': return self.model(ty['name'],value,path,depth + 1,encode)
        if kind == 'nullable': return None if value is None else self.expression(ty['inner'],value,path,depth + 1,encode)
        if kind == 'json': return self.clone_json(value,path,depth)
        if kind == 'primitive':
            name = ty['name']
            if name == 'str' and type(value) is str:
                self.text(value,path,depth); return value
            if name == 'bool' and type(value) is bool: return value
            if name in ('type(None)','types.NoneType') and value is None: return None
            if name == '_json.JsonNumber' and type(value) is J.JsonNumber:
                self.text(value.token,path,depth); return J.JsonNumber(value.token)
            if name == 'int':
                if encode and type(value) is int:
                    self.step(path,depth,max(1,value.bit_length() // 3 + 1))
                    return J.JsonNumber(J.stringify_json(value,self.json_limits))
                if not encode and type(value) is J.JsonNumber:
                    self.text(value.token,path,depth)
                    try: return value.to_int(4096)
                    except J.JsonError as error: raise CodecError('resource' if error.kind == J.RESOURCE_LIMIT else 'conversion',self.source,path,error.message) from error
                if not encode and type(value) is int: return value
            return self.fail(path,'native scalar does not match its representation')
        if kind == 'literal':
            wire = self.clone_json(value,path,depth)
            for literal in ty['values']:
                candidate = self.clone_json(literal,path,depth)
                if self.equal(wire,candidate,path):
                    return wire if encode else literal
            return self.fail(path,'value is outside the declared literal set')
        if kind == 'union':
            for alternative in ty['alternatives']:
                if encode:
                    try: candidate = self.expression(alternative['type'],value,path,depth + 1,True)
                    except CodecError as error:
                        if error.kind == 'resource': raise
                        continue
                else: candidate = value
                try: self.validation.check(alternative['root'],candidate)
                except V.ValidationError as error:
                    if error.kind == 'evaluation_failure': raise CodecError('resource',error.source,path,error.message) from error
                    continue
                return candidate if encode else self.expression(alternative['type'],candidate,path,depth + 1,False)
            return self.fail(path,'no source branch matches this native value')
        if kind in ('list','map'):
            if type(value) is not (list if kind == 'list' else dict): return self.fail(path,'wrong container type')
            marker = id(value)
            if marker in self.active: return self.fail(path,'cyclic native value')
            self.active.add(marker)
            try:
                if kind == 'list': return [self.expression(ty['inner'],v,_child(path,str(i)),depth + 1,encode) for i,v in enumerate(value)]
                output = {}
                for key,item in value.items():
                    if type(key) is not str: return self.fail(path,'object key is not a string')
                    self.text(key,path,depth)
                    output[key] = self.expression(ty['inner'],item,_child(path,key),depth + 1,encode)
                return output
            finally: self.active.remove(marker)
        return self.fail(path,'unknown conversion instruction')
    def model(self, name: str, value: Any, path: str, depth: int, encode: bool) -> Any:
        self.step(path,depth)
        descriptor = self.plan['models'][name]
        previous = self.source,self.location
        source = descriptor['source']; self.location = source; self.source = source['document'] + '#' + source['pointer']
        try:
            if not encode: self.check(descriptor['root'],value)
            if descriptor['kind'] == 'alias': result = self.expression(descriptor['type'],value,path,depth + 1,encode)
            else:
                cls = getattr(M,name)
                if type(value) is not (cls if encode else dict): return self.fail(path,'value has the wrong native model type')
                marker = id(value)
                if marker in self.active: return self.fail(path,'cyclic native model')
                self.active.add(marker)
                try:
                    known = {field['wire'] for field in descriptor['fields']}
                    output: dict[str, Any] = {}
                    for field in descriptor['fields']:
                        item = getattr(value,field['name']) if encode else value.get(field['wire'],M.UNSET)
                        if item is M.UNSET:
                            if field['required']: return self.fail(path,'required native field is absent')
                            continue
                        converted = self.expression(field['type'],item,_child(path,field['wire']),depth + 1,encode)
                        if encode or not field['fixed']: output[field['wire'] if encode else field['name']] = converted
                    extras = value._extra_fields if encode and descriptor['extras'] is not None else {} if encode else {key:item for key,item in value.items() if key not in known}
                    extra_values = {}
                    for key,item in extras.items():
                        if type(key) is not str or key in known: return self.fail(path,'invalid or colliding extra key')
                        if descriptor['extras'] is None: return self.fail(path,'closed model contains extra fields')
                        self.text(key,path,depth)
                        extra_values[key] = self.expression(descriptor['extras'],item,_child(path,key),depth + 1,encode)
                    if encode:
                        output.update(extra_values);result = output
                    else:
                        result = cls(**output)
                        if descriptor['extras'] is not None: result._extra_fields.update(extra_values)
                finally: self.active.remove(marker)
            if encode: self.check(descriptor['root'],result)
            return result
        finally: self.source,self.location = previous

def _child(path: str, key: str) -> str:
    return path + '/' + key.replace('~','~0').replace('/','~1')

class ModelCodec(Generic[T]):
    """A typed source-bound codec; every call starts fresh finite budgets."""
    def __init__(self,name: str) -> None: self._name = name
    def decode(self,text: str | bytes | bytearray | memoryview) -> T:
        context = _Context(self._name)
        return cast(T,context.model(self._name,J.parse_json(text,context.json_limits),'',0,False))
    def decode_value(self,value: J.JsonValue) -> T:
        context = _Context(self._name)
        return cast(T,context.model(self._name,context.clone_json(value),'',0,False))
    def encode(self,value: T) -> str:
        context = _Context(self._name)
        return J.stringify_json(context.model(self._name,value,'',0,True),context.json_limits)
    def encode_value(self,value: T) -> J.JsonValue:
        context = _Context(self._name)
        return cast(J.JsonValue,context.model(self._name,value,'',0,True))
