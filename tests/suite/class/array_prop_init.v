// Fixed-size array properties with assignment pattern initializers (IEEE 1800-2017 8.4,
// 10.9.1): positional (ascending and descending ranges), `default:`, two dimensions.
class C;
  logic [31:0] up [4:7] = '{1, 2, 3, 4};
  logic [31:0] down [7:4] = '{1, 2, 3, 4};
  int d [3] = '{default: 5};
  int m [2][2] = '{'{1, 2}, '{3, 4}};
  int k [3] = '{1: 9, default: 0};
endclass
module t;
  initial begin
    C c = new;
    if (c.up[4] == 1 && c.up[7] == 4 && c.down[7] == 1 && c.down[4] == 4 && c.d[0] == 5 && c.d[2] == 5
        && c.m[0][1] == 2 && c.m[1][0] == 3 && c.k[1] == 9 && c.k[0] == 0)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
