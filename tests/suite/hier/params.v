// Parameters (IEEE 1800 6.20, 23.10): defaults, overrides by position and name, defparam,
// localparam derived from parameters, typed and ranged parameters, $bits of parameters
module leaf #(parameter W = 4, parameter [7:0] INIT = 8'h11, parameter signed S = -1)
             (output [W-1:0] o);
  localparam DOUBLE = 2 * W;
  assign o = INIT[W-1:0];
  initial t.got(W, DOUBLE, S);
endmodule
module mid;
  leaf #(6) a();
  leaf #(.INIT(8'h3c), .W(8)) b();
  leaf c();
  defparam c.W = 2;
endmodule
module t;
  integer errors = 0, n = 0;
  integer ws [0:3], ds [0:3];
  parameter integer P = 5;
  parameter real R = 2.5;
  localparam [3:0] L = P + 10;          // truncated to 4 bits
  mid m();
  leaf #(.S(3)) d();
  task got(input integer w, input integer d, input integer s);
    ws[n] = w; ds[n] = d; n = n + 1;
  endtask
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0d want %0d", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1;
    check(`__LINE__, n, 4);
    check(`__LINE__, m.a.W, 6);
    check(`__LINE__, m.a.DOUBLE, 12);
    check(`__LINE__, m.b.W, 8);
    check(`__LINE__, m.b.o, 8'h3c);
    check(`__LINE__, m.c.W, 2);           // defparam
    check(`__LINE__, m.c.o, 2'b01);
    check(`__LINE__, $bits(m.a.o), 6);
    check(`__LINE__, L, 15);
    check(`__LINE__, d.S, 3);
    check(`__LINE__, m.a.S < 0, 1);       // signed parameter
    if (R * 2 != 5.0) begin $display("FAILED line %0d", `__LINE__); errors++; end
    if (errors == 0) $display("PASSED");
  end
endmodule
