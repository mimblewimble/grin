import sys
import hashlib
from sage.all import *

for t in xrange(1023, -1, -1):
	p = 2**256 -2**32 - t
	if p.is_prime():
		print '%x'%p
		break
a = 0
b = 7
F = FiniteField(p)

o = '%x' % (EllipticCurve ([F (a), F(b)]).order())
print o 

gen2 =  hashlib.sha256('0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8'.decode('hex'))
gH = EllipticCurve ([F (0), F(7)]).lift_x(int(gen2.hexdigest(),16))
print 'gH: %s' % gH
gen_h = '%x %x'%gH.xy()
print 'GENERATOR H: %s'%gen_h

#this doesn't create a point on the curve
#gen_j_input = '04%x%x'%gG.xy()

#this does
gen_j_input = gen2.hexdigest()

print 'gen_j_input: %s'%gen_j_input
gen3 =  hashlib.sha256(gen_j_input.decode('hex'))
gJ = EllipticCurve ([F (0), F(7)]).lift_x(int(gen3.hexdigest(),16))
print 'gJ: %s' % gJ
gen_j = '%x %x'%gJ.xy()
print 'GENERATOR J: %s'%gen_j
