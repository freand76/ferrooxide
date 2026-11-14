// SystemVerilog data types: typedef, enum, packed struct/union, casts, positional assignment
// patterns (the reference has no named or default patterns: tests/sv/patterns.v)
typedef logic [7:0] byte_t;                 // file scope ($unit)
typedef enum logic [1:0] {IDLE, RUN, DONE = 3} state_t;

typedef struct packed {
  logic [3:0] tag;
  byte_t      data;
  logic       valid;
} pkt_t;

typedef struct packed {
  pkt_t       p;
  logic [1:0][3:0] nib;
  int         count;
} rec_t;

module sink(input pkt_t in, output byte_t dbl);
  assign dbl = {in.data[6:0], 1'b0};
endmodule

module tb;
  typedef union packed { logic [15:0] w; logic [1:0][7:0] b; } u16_t;
  localparam pkt_t P0 = '{4'h5, 8'hA1, 1'b1};
  state_t st, nx;
  pkt_t pk;
  rec_t r;
  pkt_t arr [0:3];
  pkt_t [1:0] pair;
  u16_t u;
  byte_t d2;
  int i;
  sink s(.in(pk), .dbl(d2));

  function automatic pkt_t mk(input logic [3:0] t, input byte_t d);
    mk = '{t, d, 1'b1};
  endfunction

  always_comb begin
    case (st)
      IDLE: nx = RUN;
      RUN: nx = DONE;
      default: nx = IDLE;
    endcase
  end

  initial begin
    $display("bits %0d %0d %0d %0d", $bits(pkt_t), $bits(rec_t), $bits(state_t), $bits(u16_t));
    pk = P0;
    $display("P0 %h tag %h data %h valid %b", P0, pk.tag, pk.data, pk.valid);
    st = IDLE;
    #1 $display("st %0d nx %0d RUN=%0d DONE=%0d", st, nx, RUN, DONE);
    st = nx;
    #1 $display("st %0d nx %0d", st, nx);
    pk = '0;
    pk.tag = 4'hC;
    pk.data = 8'h3F;
    pk.valid = 1;
    #1 $display("pk %h tag %h data %h d2 %h", pk, pk.tag, pk.data, d2);
    pk.data[7:4] = 4'h9;
    pk.data[0] = 0;
    $display("pk %h data %h bit3 %b", pk, pk.data, pk.data[3]);
    r = '{P0, '{4'h1, 4'h2}, -5};
    $display("r %h p.data %h nib1 %h nib0 %h count %0d", r, r.p.data, r.nib[1], r.nib[0], r.count);
    r.p.tag = 4'hF;
    r.nib[0] = 4'h7;
    r.count = r.count + 10;
    $display("r %h count %0d", r, r.count);
    r = '0;
    $display("r %h", r);
    for (i = 0; i < 4; i = i + 1)
      arr[i] = mk(i, 8'h10 * i + 1);
    for (i = 0; i < 4; i = i + 1)
      $display("arr[%0d] %h tag %h data %h", i, arr[i], arr[i].tag, arr[i].data);
    arr[2].data = 8'hEE;
    i = 1;
    arr[2].tag = 4'h0;
    $display("arr[2] %h", arr[2]);
    pair[0] = mk(1, 8'h22);
    pair[1] = mk(2, 8'h33);
    i = 0;
    $display("pair %h p1.data %h pi.tag %h", pair, pair[1].data, pair[i].tag);
    pair[0].data = 8'h44;
    $display("pair %h", pair);
    u.w = 16'hBEEF;
    $display("u %h b1 %h b0 %h", u, u.b[1], u.b[0]);
    u.b[0] = 8'h01;
    $display("u %h", u.w);
    $display("casts %h %h %0d %0d %h", 4'(8'hAB), byte_t'(12'hFED), int'(3.7), signed'(4'hF), 12'(4'hF + 4'h1));
    $display("st cast %0d", state_t'(2));
    $finish;
  end
endmodule
