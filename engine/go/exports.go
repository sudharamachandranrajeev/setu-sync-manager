package main

/*
#include <stdlib.h>
*/
import "C"

import (
	"encoding/json"
	"unsafe"
)

// Every export returns a JSON text Setu must give back to setu_engine_free:
// {"ok": <result>} or {"err": "<message>"}.

func reply(v any, err error) *C.char {
	var out []byte
	if err != nil {
		out, _ = json.Marshal(map[string]string{"err": err.Error()})
	} else if out, err = json.Marshal(map[string]any{"ok": v}); err != nil {
		out, _ = json.Marshal(map[string]string{"err": "engine reply: " + err.Error()})
	}
	return C.CString(string(out))
}

// setu_engine_start starts the engine. args: StartArgs as JSON. ok: {"myID": "..."}.
//
//export setu_engine_start
func setu_engine_start(args *C.char) *C.char {
	var a StartArgs
	if err := json.Unmarshal([]byte(C.GoString(args)), &a); err != nil {
		return reply(nil, err)
	}
	return reply(start(a))
}

// setu_engine_stop stops syncing and closes the database. It returns when everything has
// stopped.
//
//export setu_engine_stop
func setu_engine_stop() *C.char {
	return reply(nil, stop())
}

// setu_engine_call runs one call (see calls.go). args is a JSON object, or empty.
//
//export setu_engine_call
func setu_engine_call(method, args *C.char) *C.char {
	return reply(call(C.GoString(method), []byte(C.GoString(args))))
}

//export setu_engine_free
func setu_engine_free(p *C.char) {
	C.free(unsafe.Pointer(p))
}

func main() {}
