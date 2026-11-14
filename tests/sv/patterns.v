// SystemVerilog features the reference simulator lacks (checked against tests/sv/*.expected):
// named and default assignment patterns, struct members of array elements written at run-time
// indexes, member access of struct parameters.
typedef struct packed {
  logic [3:0] tag;
  logic [7:0] data;
  logic       valid;
} pkt_t;

module tb;
  localparam pkt_t P0 = '{tag: 4'h5, data: 8'hA1, valid: 1'b1};
  pkt_t p, arr [0:3];
  pkt_t [1:0] pair;
  logic [3:0][7:0] w;
  int i;

  initial begin
    $display("P0 %h tag %h data %h valid %b", P0, P0.tag, P0.data, P0.valid);
    p = '{valid: 1, default: 0};
    $display("p %h", p);
    p = '{data: 8'hFF, tag: 4'h1, valid: 0};
    $display("p %h", p);
    p = '{default: '1};
    $display("p %h", p);
    w = '{default: 8'h5A};
    $display("w %h", w);
    w = '{8'h01, 8'h02, 8'h03, 8'h04};
    $display("w %h w[0] %h", w, w[0]);
    for (i = 0; i < 4; i = i + 1)
      arr[i] = '{tag: i, data: 8'h10 * i, valid: i[0]};
    i = 1;
    arr[i+1].tag = 4'hF;
    arr[i].data[3:0] = 4'h7;
    for (i = 0; i < 4; i = i + 1)
      $display("arr[%0d] %h", i, arr[i]);
    pair = '0;
    i = 1;
    pair[i].data = 8'h44;
    pair[i-1].valid = 1;
    $display("pair %h", pair);
    $finish;
  end
endmodule
