// Members of struct variables and ports through hierarchical names (IEEE 1800 23.6,
// 7.2), also of a type given by a type parameter, and members of arrays of such structs;
// packed dimensions on a type parameter's type (`T [1:0] x`, 6.20.3) and struct members
// in a concurrent assertion (16.6).
typedef struct packed { logic [7:0] a; logic v; } s_t;

module leaf #(parameter type T = logic) (input clk, output T o, input T [1:0] i);
  s_t q;
  T [1:0] y;
  assign o = '{a: 8'h5a, v: 1'b1};
  initial q = '{a: 8'h77, v: 1'b1};
  assert property (@(posedge clk) i[0].v) else $display("FAILED: assertion");
endmodule

module mid #(parameter type T = logic) (input clk, input T [1:0] i);
  T x;
  leaf #(.T(T)) l (.clk(clk), .o(x), .i(i));
endmodule

module t;
  integer errors = 0;
  reg clk = 0;
  s_t [1:0] z = '{'{8'h11, 1'b0}, '{8'h22, 1'b1}};
  mid #(.T(s_t)) u (clk, z);
  wire w = u.l.o.v;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    #1 clk = 1;
    check(`__LINE__, w, 1);
    check(`__LINE__, u.l.o.a, 8'h5a);
    check(`__LINE__, u.x.a, 8'h5a);
    check(`__LINE__, u.l.q.a, 8'h77);
    check(`__LINE__, $bits(u.l.y), 18);
    check(`__LINE__, $bits(u.l.i), 18);
    check(`__LINE__, u.l.i[1].a, 8'h11);
    u.l.q.a = 8'h78;
    check(`__LINE__, u.l.q, 9'h0f1);
    #1 if (errors == 0) $display("PASSED");
  end
endmodule
