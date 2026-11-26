// Assignment patterns for part selects and elements of packed arrays (IEEE 1800 10.9.1),
// members of a struct parameter in the bounds of a part select (11.2.1), and packed
// dimensions added to a type parameter's type by a typedef, in a function argument (6.18).
typedef struct packed { int unsigned Len; int unsigned Levels; bit On; } cfg_t;

module leaf #(parameter cfg_t C = '0, parameter int W = 16) (output logic [3:0][7:0] g, input logic [3:0][7:0] t);
  localparam type be_t = logic [W/8-1:0];
  typedef be_t [3:0] buf_t;
  function automatic logic pick(input buf_t v, int i, int j);
    return v[i][j];
  endfunction
  always_comb begin
    g = '0;
    for (int unsigned i = 0; i < 4; i++)
      if (C.On) g[i][C.Len/C.Levels-1:0] = t[i];
  end
  initial #1 if ($bits(buf_t) != 8 || pick(8'b10_01_00_11, 3, 1) !== 1'b1 || pick(8'b10_01_00_11, 2, 0) !== 1'b1)
    $display("FAILED: typedef of a type parameter's type");
endmodule

module t;
  integer errors = 0;
  localparam cfg_t K = '{Len: 8, Levels: 2, On: 1};
  logic [1:0][15:0] f;
  logic [15:0] h;
  logic [3:0][7:0] g, src = 32'h44332211;
  wire b = 1'b1;
  assign f[1][11:4] = '{default: b};
  assign f[1][15:12] = '0;
  assign f[1][3:0] = 4'h0;
  assign f[0] = '{default: 1'b0};
  leaf #(.C(K)) l (g, src);
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    h = 0;
    h[7:4] = '{default: 1'b1};
    h[8 +: 2] = '{1'b1, 1'b0};
    #2;
    check(`__LINE__, f, 32'h0ff00000);
    check(`__LINE__, h, 16'h02f0);
    check(`__LINE__, g, 32'h04030201);
    if (errors == 0) $display("PASSED");
  end
endmodule
