// arrays of interface instances and interface port arrays (IEEE 1800 25.5): elements are
// selected by constant indexes; an array port connects element by element (the reference
// pairs the lowest indexes, ICARUS_NOTES 10; the test uses ranges where both rules agree)
interface bus_if;
  logic [7:0] data;
  modport src(output data);
  modport dst(input data);
endinterface
module drv(bus_if.src b, input [7:0] v);
  assign b.data = v;
endmodule
module sum3(bus_if.dst bs[3], output [9:0] total);
  assign total = bs[0].data + bs[1].data + bs[2].data;
endmodule
module pass(bus_if.dst bs[3], output [9:0] total);
  sum3 s(bs, total);                        // an array port passed on whole
endmodule
module pick(bus_if.dst p[2:0], q, output [7:0] first, output [7:0] other);
  assign first = p[0].data;                 // q: same interface type, no dimension
  assign other = q.data;
endmodule
module t;
  integer errors = 0;
  bus_if ifs[3]();
  bus_if down[2:0]();
  bus_if one();
  wire [9:0] tot, tot2;
  wire [7:0] f, o;
  genvar g;
  for (g = 0; g < 3; g++) begin : gen
    drv d(ifs[g], 8'd10 + g);
    drv e(down[g], 8'd20 + g);
  end
  drv d1(one, 8'd99);
  sum3 s(ifs, tot);
  pass p(.bs(down), .total(tot2));
  pick k(down, one, f, o);
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, ifs[1].data, 11);
    check(`__LINE__, tot, 33);
    check(`__LINE__, tot2, 63);
    check(`__LINE__, f, 20);                // p[0] is down[0]
    check(`__LINE__, o, 99);
    if (errors == 0) $display("PASSED");
  end
endmodule
