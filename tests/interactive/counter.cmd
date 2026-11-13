watch q d
break q 8'd5
run 200
delete
unwatch q
run 250
print q d
print clk b
force q 8'h80
print msb
step
print msb b
run +20ns
print q d
release q
run +20
p q d
set rst 1
run +20
p q d
set rst 0
run +32
p q
ls u1
print nope
break msb 1
info
run
time
quit
