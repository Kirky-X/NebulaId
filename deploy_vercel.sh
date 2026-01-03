#!/bin/bash
set -e

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}Nebula ID - Vercel Deployment Helper${NC}"
echo "======================================"

# Check if Vercel CLI is installed
if ! command -v vercel &> /dev/null; then
    echo -e "${YELLOW}Vercel CLI not found. Installing...${NC}"
    npm install -g vercel
else
    echo -e "${GREEN}Vercel CLI is already installed.${NC}"
fi

# Build verification
echo -e "${YELLOW}Verifying build...${NC}"
cargo check -p nebula-api
if [ $? -eq 0 ]; then
    echo -e "${GREEN}Build verification passed.${NC}"
else
    echo -e "${RED}Build verification failed!${NC}"
    exit 1
fi

echo -e "${YELLOW}Starting deployment...${NC}"
echo "You will be prompted to log in to Vercel if you haven't already."
echo "Press Enter to continue..."
read

# Deploy
vercel

echo -e "${GREEN}Deployment process completed!${NC}"
echo "Please check the Vercel dashboard for build logs and URL."
