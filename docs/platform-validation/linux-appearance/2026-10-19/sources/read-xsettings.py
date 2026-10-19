import ctypes,struct
x=ctypes.CDLL('libX11.so.6');x.XOpenDisplay.restype=ctypes.c_void_p
x.XOpenDisplay.argtypes=[ctypes.c_char_p];d=x.XOpenDisplay(None)
x.XInternAtom.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_int];x.XInternAtom.restype=ctypes.c_ulong
x.XGetSelectionOwner.argtypes=[ctypes.c_void_p,ctypes.c_ulong];x.XGetSelectionOwner.restype=ctypes.c_ulong
owner=x.XGetSelectionOwner(d,x.XInternAtom(d,b'_XSETTINGS_S0',0));print('owner',owner)
x.XGetWindowProperty.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_ulong,ctypes.c_long,ctypes.c_long,ctypes.c_int,ctypes.c_ulong,ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.POINTER(ctypes.c_ubyte))]
t=ctypes.c_ulong();f=ctypes.c_int();n=ctypes.c_ulong();left=ctypes.c_ulong();p=ctypes.POINTER(ctypes.c_ubyte)()
x.XGetWindowProperty(d,owner,x.XInternAtom(d,b'_XSETTINGS_SETTINGS',0),0,100000,0,0,ctypes.byref(t),ctypes.byref(f),ctypes.byref(n),ctypes.byref(left),ctypes.byref(p))
b=bytes(p[:n.value]);e='<' if b[0]==0 else '>';total=struct.unpack_from(e+'I',b,8)[0];o=12
for _ in range(total):
 kind=b[o];length=struct.unpack_from(e+'H',b,o+2)[0];o+=4;name=b[o:o+length].decode();o=(o+length+3)//4*4;o+=4
 if kind==0:value=struct.unpack_from(e+'i',b,o)[0];o+=4
 elif kind==1:
  length=struct.unpack_from(e+'I',b,o)[0];o+=4;value=b[o:o+length].decode();o=(o+length+3)//4*4
 else:value=struct.unpack_from(e+'HHHH',b,o);o+=8
 if 'Theme' in name or 'Dark' in name:print(name,repr(value))
