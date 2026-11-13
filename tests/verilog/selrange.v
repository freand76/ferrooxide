// Out-of-range and x/z constant selects (read x, writes skipped), in all execution modes
module top;
  reg [2:1] e;
  reg [7:0] in;
  reg [3:0] v, o;
  reg [5:0] c;
  reg [7:0] mem [0:1];
  integer r;
  initial begin
    e = 2'b10; in = 8'hA5; v = 0;
    mem[0] = 0; mem[1] = 0;
    $display("%b %b %b", e[1:0], e[11:6], {e[1:0], 4'b0001});
    $display("%b", in[7:'dx]);
    o = in[7:'dx];
    $display("%b", o);
    v[1'bx:0] = 1'b1;
    v[1'bx] = 1'b1;
    v[5:2] = 4'b1111;
    $display("%b", v);
    mem[1][9:6] = 4'b1111;
    mem['bx] = 8'hff;
    $display("%b %b", mem[0], mem[1]);
    c = 6'b01_xx01;
    casex (c)
      {e[1:0], 4'b??01} : r = 3;
      default: r = 0;
    endcase
    $display("%0d", r);
  end
endmodule
