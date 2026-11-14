// exit: 1
// an interface port array connected to an array of another size: an error
interface bus_if;
  logic v;
endinterface
module m(bus_if b[2]);
endmodule
module t;
  bus_if x[3]();
  m u(x);
endmodule
