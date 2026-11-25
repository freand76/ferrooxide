// expected
// arrays of interface instances: elements connected by constant and genvar indexes
interface bus_if;
  logic [7:0] data;
  logic valid;
  modport src(output data, valid);
endinterface
module drv(bus_if b, input [7:0] v);
  assign b.data = v;
  assign b.valid = 1;
endmodule
module t;
  bus_if ifs[3]();
  bus_if one();
  drv d0(ifs[0], 8'd10);
  drv d1(.b(ifs[1]), .v(8'd11));
  drv d2(one, 8'd5);
  genvar g;
  for (g = 0; g < 3; g++) begin : gen
    initial #1 $display("%0d %0d %b", g, ifs[g].data, ifs[g].valid);
  end
  initial begin
    ifs[2].data = 42;
    #2 $display("%0d %0d", one.data, ifs[2].data);
  end
endmodule
